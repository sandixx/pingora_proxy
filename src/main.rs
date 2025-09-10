use dotenvy::dotenv;
use log::{info, warn};
use pingora_core::server::configuration::Opt;
use pingora_core::server::Server;
use pingora_proxy::http_proxy_service;
use std::sync::Arc;
use structopt::StructOpt;
use tokio::sync::RwLock;

mod backend;
mod config;
mod health_check;
mod proxy;

use config::*;
use health_check::HealthChecker;
use proxy::MyProxy;

#[derive(StructOpt, Debug)]
#[structopt(name = "pingora-proxy")]
struct Args {
    #[structopt(short = "p", long = "port")]
    proxy_port: Option<u16>,

    #[structopt(short = "c", long = "conf", help = "Path to configuration file")]
    conf: Option<String>,
}

fn main() {
    dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::from_args();

    let proxy_port = get_proxy_port(args.proxy_port);
    let ssl_enabled = is_ssl_enabled();
    let ssl_cert = get_ssl_cert();
    let ssl_key = get_ssl_key();

    let (backends, round_robin) = load_backends(); // Updated to return tuple
    let custom_headers = load_custom_headers();
    let remove_headers = load_remove_headers();
    let health_check_config = load_health_check_config();

    // Convert backends to thread-safe structure
    let shared_backends = Arc::new(RwLock::new(backends));

    // Create a runtime for the health check service
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    // Start health check service in a separate thread
    let health_backends = shared_backends.clone();
    let health_config = health_check_config.clone();
    std::thread::spawn(move || {
        rt.block_on(async move {
            HealthChecker::health_check_loop(health_backends, health_config).await;
        });
    });

    info!("🔍 Testing initial connection to upstreams...");
    {
        // Use a simple blocking check for initial connection test
        for b in shared_backends.blocking_read().iter() {
            match std::net::TcpStream::connect(format!("{}:{}", b.host, b.port)) {
                Ok(_) => info!("✅ {}:{} is reachable", b.host, b.port),
                Err(e) => {
                    warn!(
                        "⚠️ Cannot connect to upstream {}:{}: {} (will be marked unhealthy)",
                        b.host, b.port, e
                    );
                    // Mark as unhealthy in the shared state
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
        ssl_enabled,
        custom_headers,
        remove_headers,
        counter: std::sync::atomic::AtomicUsize::new(0),
        round_robin, // Add this field
    };

    let mut proxy_service = http_proxy_service(&my_server.configuration, proxy);

    if ssl_enabled {
        if !std::path::Path::new(&ssl_cert).exists() {
            panic!("SSL certificate not found: {}", ssl_cert);
        }
        if !std::path::Path::new(&ssl_key).exists() {
            panic!("SSL private key not found: {}", ssl_key);
        }

        info!("🔒 SSL/TLS enabled");
        proxy_service
            .add_tls(&format!("0.0.0.0:{}", proxy_port), &ssl_cert, &ssl_key)
            .unwrap();
    } else {
        info!("🔓 SSL/TLS disabled - using HTTP");
        proxy_service.add_tcp(&format!("0.0.0.0:{}", proxy_port));
    }

    my_server.add_service(proxy_service);

    info!("🚀 Starting Pingora Proxy Server with Health Checks");
    my_server.run_forever();
}