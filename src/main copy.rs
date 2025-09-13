use log::{info, warn};
use pingora_core::server::configuration::Opt;
use pingora_core::server::Server;
use pingora_proxy::http_proxy_service;
use pingora_core::listeners::tls::TlsSettings;

use std::sync::{Arc, Mutex};
use std::{process, thread};
use std::time::Duration;
use structopt::StructOpt;
use tokio::sync::RwLock;

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
    TlsSettings::intermediate(cert_path, key_path)
        .expect("Failed to create TlsSettings")
}

fn main() {
    dotenvy::from_filename(".env").expect("⚠️ .env file not found!");
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::from_args();

    let proxy_port = get_proxy_port(args.proxy_port);
    let ssl = is_ssl_enabled();

    if ssl.status {
        loop {
            let day_cert = check_cert();

            if !day_cert.is_good {
                warn!("{}", day_cert.error);
                process::exit(1);
            }

            if day_cert.day_left <= 1 {

            }

            // Sleep for 24 hours
            thread::sleep(Duration::from_secs(60 * 60 * 24));
        }
    }

    let backends = load_backends();
    let custom_headers = load_custom_headers();
    let remove_headers = load_remove_headers();
    let health_check_config = load_health_check_config();
    let load_balance_strategy = load_balance_strategy();
    let sticky_cookie_name = load_sticky_cookie_name();
    let sticky_session_ttl = config::load_sticky_session_ttl();

    let shared_backends = Arc::new(RwLock::new(backends));
    let load_balancer = Arc::new(LoadBalancer::new(load_balance_strategy));

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let health_backends = shared_backends.clone();
    let health_config = health_check_config.clone();
    std::thread::spawn(move || {
        rt.block_on(async move {
            HealthChecker::health_check_loop(health_backends, health_config).await;
        });
    });

    info!("🔍 Testing initial connection to upstreams...");
    {
        for b in shared_backends.blocking_read().iter() {
            match std::net::TcpStream::connect(format!("{}:{}", b.host, b.port)) {
                Ok(_) => info!("✅ {}:{} is reachable", b.host, b.port),
                Err(e) => {
                    warn!(
                        "⚠️ Cannot connect to upstream {}:{}: {} (will be marked unhealthy)",
                        b.host, b.port, e
                    );
                    
                    let mut backends_write = shared_backends.blocking_write();
                    if let Some(backend) = backends_write.iter_mut().find(|be| be.host == b.host && be.port == b.port) {
                        backend.healthy = false;
                    }
                }
            }
        }
    }

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
        backends: shared_backends,
        load_balancer,
        ssl_enabled: ssl.status,
        custom_headers,
        remove_headers,
        sticky_cookie_name,
        sticky_session_ttl,
    };

    let mut proxy_service = http_proxy_service(&my_server.configuration, proxy);

    if ssl.status {
        if !std::path::Path::new(&ssl.cert_loc).exists() {
            panic!("SSL certificate not found: {}", ssl.cert_loc);
        }
        if !std::path::Path::new(&ssl.key_loc).exists() {
            panic!("SSL private key not found: {}", ssl.key_loc);
        }

        info!("🔒 SSL/TLS enabled");
        proxy_service
            .add_tls(&format!("0.0.0.0:{}", proxy_port), &ssl.cert_loc, &ssl.key_loc)
            .unwrap();
    } else {
        info!("🔓 SSL/TLS disabled - using HTTP");
        proxy_service.add_tcp(&format!("0.0.0.0:{}", proxy_port));
    }

    my_server.add_service(proxy_service);

    info!("🚀 Starting Pingora Proxy Server with Health Checks");
    my_server.run_forever();
}