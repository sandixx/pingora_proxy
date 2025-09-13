use log::{info, warn};
use pingora_core::server::configuration::Opt;
use pingora_core::server::Server;
use pingora_proxy::http_proxy_service;
use pingora_core::listeners::tls::TlsSettings;
use std::sync::{Arc, Mutex, RwLock};
use std::{process, thread};
use std::time::Duration;
use structopt::StructOpt;

mod backend;
mod config;
mod health_check;
mod load_balancer;
mod proxy;
mod ssl_watcher;

use config::*;
use health_check::HealthChecker;
use load_balancer::LoadBalancer;
use proxy::MyProxy;
use ssl_watcher::check_cert;

#[derive(StructOpt, Debug)]
#[structopt(name = "pingora-proxy")]
struct Args {
    #[structopt(short = "p", long = "port")]
    proxy_port: Option<u16>,

    #[structopt(short = "c", long = "conf", help = "Path to configuration file")]
    conf: Option<String>,
}

fn load_tls_settings(cert_path: &str, key_path: &str) -> TlsSettings {
    if !std::path::Path::new(cert_path).exists() {
        panic!("SSL certificate not found: {}", cert_path);
    }
    if !std::path::Path::new(key_path).exists() {
        panic!("SSL private key not found: {}", key_path);
    }
    
    match TlsSettings::intermediate(cert_path, key_path) {
        Ok(settings) => settings,
        Err(e) => {
            warn!("Failed to load TLS settings: {}, regenerating SSL...", e);
            
            let gen_ssl = generate_ssl();
            if gen_ssl.status != "Success" {
                panic!("Failed to regenerate SSL: {}", gen_ssl.error);
            }
            
            TlsSettings::intermediate(cert_path, key_path)
                .expect("Failed to create TlsSettings even after SSL regeneration")
        }
    }
}

fn main() {
    dotenvy::from_filename(".env").expect("⚠️ .env file not found!");
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::from_args();

    let proxy_port = get_proxy_port(args.proxy_port);
    let ssl = is_ssl_enabled();

    let cert_path = ssl.cert_loc.clone();
    let key_path = ssl.key_loc.clone();

    let shared_tls = if ssl.status {
        if !std::path::Path::new(&ssl.cert_loc).exists() {
            panic!("SSL certificate not found: {}", ssl.cert_loc);
        }
        if !std::path::Path::new(&ssl.key_loc).exists() {
            panic!("SSL private key not found: {}", ssl.key_loc);
        }
        Some(Arc::new(Mutex::new(load_tls_settings(&ssl.cert_loc, &ssl.key_loc))))
    } else {
        None
    };

    if let Some(tls_arc) = shared_tls.clone() {
        let cert_path = cert_path.clone();
        let key_path = key_path.clone();
        thread::spawn(move || {
            let mut signals =
                signal_hook::iterator::Signals::new(&[signal_hook::consts::signal::SIGHUP])
                    .expect("Failed to bind signals");
            for _ in signals.forever() {
                info!("SIGHUP received: reloading TLS cert...");
                let new_settings = load_tls_settings(&cert_path, &key_path);
                *tls_arc.lock().unwrap() = new_settings;
            }
        });
    }

    if let Some(tls_arc) = shared_tls.clone() {
        let cert_path = cert_path.clone();
        let key_path = key_path.clone();
        thread::spawn(move || {
            loop {
                let day_cert = check_cert();
                if !day_cert.is_good {
                    warn!("{}", day_cert.error);
                    process::exit(1);
                }
                if day_cert.day_left <= 1 {
                    warn!("⚠️ Cert about to expire, reloading...");
                    let gen_ssl = generate_ssl();

                    if gen_ssl.status != "Success".to_string() {
                        warn!("{}", gen_ssl.error);
                        process::exit(1);
                    }
                    
                    let new_settings = load_tls_settings(&cert_path, &key_path);
                    *tls_arc.lock().unwrap() = new_settings;
                }
                thread::sleep(Duration::from_secs(60 * 60 * 24));
            }
        });
    }

    let backends = load_backends();
    let custom_headers = load_custom_headers();
    let remove_headers = load_remove_headers();
    let health_check_config = load_health_check_config();
    let load_balance_strategy = load_balance_strategy();
    let sticky_cookie_name = load_sticky_cookie_name();
    let sticky_session_ttl = config::load_sticky_session_ttl();

    let shared_backends_std = Arc::new(RwLock::new(backends));
    let load_balancer = Arc::new(LoadBalancer::new(load_balance_strategy));

    info!("🔍 Testing initial connection to upstreams...");
    let shared_backends_std_clone = shared_backends_std.clone();
    let health_check_handle = thread::spawn(move || {
        let backends_guard = shared_backends_std_clone.read().unwrap();
        let mut unhealthy_backends = Vec::new();
        
        for b in backends_guard.iter() {
            match std::net::TcpStream::connect(format!("{}:{}", b.host, b.port)) {
                Ok(_) => info!("✅ {}:{} is reachable", b.host, b.port),
                Err(e) => {
                    warn!(
                        "⚠️ Cannot connect to upstream {}:{}: {} (will be marked unhealthy)",
                        b.host, b.port, e
                    );
                    unhealthy_backends.push((b.host.clone(), b.port));
                }
            }
        }
        
        drop(backends_guard);
        
        if !unhealthy_backends.is_empty() {
            let mut backends_write = shared_backends_std_clone.write().unwrap();
            for (host, port) in unhealthy_backends {
                if let Some(backend) = backends_write.iter_mut().find(|be| be.host == host && be.port == port) {
                    backend.healthy = false;
                }
            }
        }
        
        shared_backends_std_clone.read().unwrap().clone()
    });

    let initial_backends = health_check_handle.join().unwrap();

    let shared_backends = Arc::new(RwLock::new(initial_backends));

    let health_backends = shared_backends.clone();
    let health_config = health_check_config.clone();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            HealthChecker::health_check_loop(health_backends, health_config).await;
        });
    });

    let server_opt = args.conf.map(|conf_path| Opt {
        upgrade: false,
        daemon: false,
        nocapture: false,
        test: false,
        conf: Some(conf_path),
    });

    let mut my_server = Server::new(server_opt).unwrap();
    my_server.bootstrap();

    let proxy = MyProxy {
        backends: shared_backends.clone(),
        load_balancer,
        ssl_enabled: ssl.status,
        custom_headers,
        remove_headers,
        sticky_cookie_name,
        sticky_session_ttl,
    };

    let mut proxy_service = http_proxy_service(&my_server.configuration, proxy);

    if ssl.status {
        info!("🔒 Starting TLS listener on {}", proxy_port);
        
        let tls_settings = load_tls_settings(&cert_path, &key_path);
        
        proxy_service.add_tls_with_settings(
            &format!("0.0.0.0:{}", proxy_port),
            None,
            tls_settings,
        );
    } else {
        info!("🔓 Starting plain TCP listener on {}", proxy_port);
        proxy_service.add_tcp(&format!("0.0.0.0:{}", proxy_port));
    }

    my_server.add_service(proxy_service);
    info!("🚀 Starting Pingora Proxy Server with Health Checks");
    my_server.run_forever();
}