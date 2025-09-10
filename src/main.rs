use async_trait::async_trait;
use dotenvy::dotenv;
use log::{info, warn, error};
use pingora_core::server::configuration::Opt;
use pingora_core::server::Server;
use pingora_core::upstreams::peer::HttpPeer;
use pingora_core::Result;
use pingora_http::ResponseHeader;
use pingora_proxy::{http_proxy_service, ProxyHttp, Session};
use std::collections::HashMap;
use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use structopt::StructOpt;

#[derive(Clone, Debug)]
pub struct Backend {
    pub host: String,
    pub port: u16,
    pub weight: usize,
    pub healthy: bool,
    pub last_checked: Option<Instant>,
}

#[derive(Clone)]
pub struct HealthCheckConfig {
    pub enabled: bool,
    pub path: String,
    pub interval_secs: u64,
    pub timeout_secs: u64,
    pub success_codes: Vec<u16>,
}

pub struct MyProxy {
    backends: Arc<RwLock<Vec<Backend>>>,
    ssl_enabled: bool,
    custom_headers: HashMap<String, String>,
    remove_headers: Vec<String>,
    counter: AtomicUsize,
    // health_check_config: HealthCheckConfig,
}

impl MyProxy {
    async fn select_backend(&self) -> Option<Backend> {
        let backends = self.backends.read().await;
        let healthy_backends: Vec<&Backend> = backends.iter().filter(|b| b.healthy).collect();
        
        if healthy_backends.is_empty() {
            warn!("⚠️ No healthy backends available, falling back to all backends");
            // Fallback to all backends if none are healthy
            let total_weight: usize = backends.iter().map(|b| b.weight).sum();

            if total_weight == 0 {
                return backends.first().cloned();
            }
            
            let choice = (self.counter.fetch_add(1, Ordering::Relaxed) % 100) as usize;
            let mut acc = 0;
            
            for b in backends.iter() {
                acc += b.weight;
                if choice < acc {
                    return Some(b.clone());
                }
            }
            return backends.first().cloned();
        }

        let total_weight: usize = healthy_backends.iter().map(|b| b.weight).sum();

        if total_weight == 0 {
            return healthy_backends.first().cloned().cloned();
        }

        let choice = (self.counter.fetch_add(1, Ordering::Relaxed) % 100) as usize;
        let mut acc = 0;
        
        // Iterate over a reference to avoid moving the vector
        for b in &healthy_backends {
            acc += b.weight;
            if choice < acc {
                return Some((*b).clone());
            }
        }
        
        healthy_backends.first().cloned().cloned()
    }

    async fn health_check_loop(backends: Arc<RwLock<Vec<Backend>>>, config: HealthCheckConfig) {
        if !config.enabled {
            info!("🩺 Health checks disabled");
            return;
        }

        info!("🩺 Starting health check service (interval: {}s)", config.interval_secs);
        
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .unwrap();

        loop {
            tokio::time::sleep(Duration::from_secs(config.interval_secs)).await;
            
            let mut backends_write = backends.write().await;
            for backend in backends_write.iter_mut() {
                let url = format!("http://{}:{}{}", backend.host, backend.port, config.path);

                match client.get(&url).send().await {
                    Ok(response) => {
                        let is_healthy = config.success_codes.contains(&response.status().as_u16());

                        if backend.healthy != is_healthy {
                            if is_healthy {
                                info!("✅ Backend {}:{} is now healthy", backend.host, backend.port);
                            } else {
                                warn!("❌ Backend {}:{} is now unhealthy", backend.host, backend.port);
                            }
                            backend.healthy = is_healthy;
                        }
                        backend.last_checked = Some(Instant::now());
                    }
                    Err(e) => {
                        if backend.healthy {
                            warn!("❌ Backend {}:{} health check failed: {}", backend.host, backend.port, e);
                            backend.healthy = false;
                        }
                        backend.last_checked = Some(Instant::now());
                    }
                }
            }
        }
    }
}

#[async_trait]
impl ProxyHttp for MyProxy {
    type CTX = ();

    fn new_ctx(&self) -> Self::CTX {
        ()
    }

    async fn request_filter(&self, session: &mut Session, _ctx: &mut Self::CTX) -> Result<bool> {
        session.req_header_mut().insert_header("X-Forwarded-By", "Pingora-Proxy")?;

        let proto = if self.ssl_enabled { "https" } else { "http" };
        session.req_header_mut().insert_header("X-Forwarded-Proto", proto)?;

        if let Some(client_addr) = session.client_addr() {
            let addr_string = client_addr.to_string();
            let client_ip = addr_string.split(':').next().unwrap_or("unknown");

            if let Some(existing_forwarded) = session.req_header().headers.get("X-Forwarded-For") {
                if let Ok(existing_str) = existing_forwarded.to_str() {
                    let new_value = format!("{}, {}", existing_str, client_ip);
                    session.req_header_mut().insert_header("X-Forwarded-For", new_value)?;
                }
            } else {
                session.req_header_mut().insert_header("X-Forwarded-For", client_ip)?;
            }

            info!("{} {} {}", session.req_header().method, client_ip, session.req_header().uri);
        }

        Ok(false)
    }

    async fn upstream_peer( &self, _session: &mut Session, _ctx: &mut Self::CTX, ) -> Result<Box<HttpPeer>> {
        match self.select_backend().await {
            Some(backend) => {
                let peer = Box::new(HttpPeer::new(format!("{}:{}", backend.host, backend.port), false, "".to_string(), ));
                Ok(peer)
            }
            None => {
                error!("🚨 No backends available for routing");
                Err(pingora_core::Error::new_str("No backends available"))
            }
        }
    }

    async fn response_filter(&self, _session: &mut Session, upstream_response: &mut ResponseHeader, _ctx: &mut Self::CTX, ) -> Result<()> {
        for key in &self.remove_headers {
            upstream_response.remove_header(key.as_str());
        }

        for (key, value) in &self.custom_headers {
            upstream_response.insert_header(key.clone(), value.clone())?;
        }

        Ok(())
    }
}

#[derive(StructOpt, Debug)]
#[structopt(name = "pingora-proxy")]
struct Args {
    #[structopt(short = "p", long = "port")]
    proxy_port: Option<u16>,

    #[structopt(short = "c", long = "conf", help = "Path to configuration file")]
    conf: Option<String>,
}

fn load_backends() -> Vec<Backend> {
    let mut backends = Vec::new();
    if let Ok(val) = env::var("BACKENDS") {
        for entry in val.split(',') {
            let parts: Vec<&str> = entry.split(':').collect();
            if parts.len() == 3 {
                if let (Ok(port), Ok(weight)) =
                    (parts[1].parse::<u16>(), parts[2].parse::<usize>())
                {
                    backends.push(Backend {
                        host: parts[0].to_string(),
                        port,
                        weight,
                        healthy: true, // Assume healthy initially
                        last_checked: None,
                    });
                }
            }
        }
    }
    if backends.is_empty() {
        panic!("❌ BACKENDS must be set and not empty!");
    }

    let total: usize = backends.iter().map(|b| b.weight).sum();
    if total != 100 {
        log::warn!("⚠️ BACKENDS weights sum to {} instead of 100 (auto-normalizing)", total);
        let factor = 100.0 / (total as f64);
        for b in backends.iter_mut() {
            b.weight = ((b.weight as f64) * factor).round() as usize;
        }
    }

    backends
}

fn load_health_check_config() -> HealthCheckConfig {
    let enabled = env::var("HEALTH_CHECK_ENABLED")
        .unwrap_or_else(|_| "true".to_string())
        .to_lowercase() == "true";

    let path = env::var("HEALTH_CHECK_PATH")
        .unwrap_or_else(|_| "/health".to_string());

    let interval_secs = env::var("HEALTH_CHECK_INTERVAL")
        .unwrap_or_else(|_| "30".to_string())
        .parse()
        .unwrap_or(30);

    let timeout_secs = env::var("HEALTH_CHECK_TIMEOUT")
        .unwrap_or_else(|_| "5".to_string())
        .parse()
        .unwrap_or(5);

    let success_codes_str = env::var("HEALTH_CHECK_SUCCESS_CODES")
        .unwrap_or_else(|_| "200".to_string());

    let success_codes: Vec<u16> = success_codes_str
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    HealthCheckConfig {
        enabled,
        path,
        interval_secs,
        timeout_secs,
        success_codes: if success_codes.is_empty() { vec![200] } else { success_codes },
    }
}

fn load_custom_headers() -> HashMap<String, String> {
    let mut headers = HashMap::new();
    if let Ok(val) = env::var("CUSTOM_HEADER") {
        match serde_json::from_str::<HashMap<String, String>>(&val) {
            Ok(map) => headers = map,
            Err(e) => {
                log::warn!(
                    "⚠️ Failed to parse CUSTOM_HEADER env: {} (value={})",
                    e,
                    val
                );
            }
        }
    }
    headers
}

fn load_remove_headers() -> Vec<String> {
    if let Ok(val) = env::var("REMOVE_HEADER") {
        match serde_json::from_str::<Vec<String>>(&val) {
            Ok(list) => list,
            Err(e) => {
                log::warn!(
                    "⚠️ Failed to parse REMOVE_HEADER env: {} (value={})",
                    e,
                    val
                );
                Vec::new()
            }
        }
    } else {
        Vec::new()
    }
}

fn main() {
    dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::from_args();

    let proxy_port: u16 = args
        .proxy_port
        .unwrap_or_else(|| env::var("PROXY_PORT").unwrap_or("3000".to_string()).parse().unwrap());

    let ssl_enabled = env::var("SSL")
        .unwrap_or_else(|_| "OFF".to_string())
        .to_uppercase()
        == "ON";

    let ssl_cert = env::var("SSL_CERT").unwrap_or_else(|_| "ssl/server.pem".to_string());
    let ssl_key = env::var("SSL_KEY").unwrap_or_else(|_| "ssl/server.key".to_string());

    let backends = load_backends();
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
            MyProxy::health_check_loop(health_backends, health_config).await;
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
        counter: AtomicUsize::new(0),
        // health_check_config,
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