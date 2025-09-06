use async_trait::async_trait;
use log::info;
use pingora_core::upstreams::peer::HttpPeer;
use pingora_core::Result;
use pingora_http::ResponseHeader;
use pingora_proxy::{ProxyHttp, Session};
use std::env;
use dotenvy::dotenv;
use pingora_core::server::configuration::Opt;

pub struct MyProxy {
    target_host: String,
    target_port: u16,
    ssl_enabled: bool,
}

impl MyProxy {
    pub fn new(target_host: String, target_port: u16, ssl_enabled: bool) -> Self {
        MyProxy {
            target_host,
            target_port,
            ssl_enabled,
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
        session
            .req_header_mut()
            .insert_header("X-Forwarded-By", "Pingora-Proxy")?;

        let proto = if self.ssl_enabled { "https" } else { "http" };
        session
            .req_header_mut()
            .insert_header("X-Forwarded-Proto", proto)?;

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

            info!(
                "{} {} {}",
                session.req_header().method,
                client_ip,
                session.req_header().uri
            );
        } else {
            info!(
                "{} {}",
                session.req_header().method,
                session.req_header().uri
            );
        }
        
        Ok(false)
    }

    async fn upstream_peer(&self, _session: &mut Session, _ctx: &mut Self::CTX) -> Result<Box<HttpPeer>> {
        let peer = Box::new(HttpPeer::new(
            format!("{}:{}", self.target_host, self.target_port),
            false,
            "".to_string(),
        ));
        Ok(peer)
    }

    async fn response_filter(&self, _session: &mut Session, upstream_response: &mut ResponseHeader, _ctx: &mut Self::CTX) -> Result<()> {
        upstream_response.insert_header("X-Proxy-Server", "Pingora")?;
        upstream_response.insert_header("X-Proxy-Version", "1.0")?;
        upstream_response.remove_header("Server");
        Ok(())
    }
}

use pingora_core::server::Server;
use pingora_proxy::http_proxy_service;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "pingora-proxy")]
struct Args {
    #[structopt(short = "p", long = "port")]
    proxy_port: Option<u16>,

    #[structopt(short = "h", long = "host")]
    target_host: Option<String>,

    #[structopt(short = "t", long = "target")]
    target_port: Option<u16>,

    #[structopt(short = "c", long = "conf", help = "Path to configuration file")]
    conf: Option<String>,
}

fn load_config() -> (u16, String, u16, bool, String, String) {
    let target_host = env::var("TARGET_HOST").expect("TARGET_HOST must be set");
    let proxy_port: u16 = env::var("PROXY_PORT").unwrap_or_else(|_| "3000".to_string()).parse::<u16>().expect("PROXY_PORT must be a valid u16 number");
    let target_port: u16 = env::var("TARGET_PORT").unwrap_or_else(|_| "8000".to_string()).parse::<u16>().expect("TARGET_PORT must be a valid u16 number");
    
    let ssl_enabled = env::var("SSL")
        .unwrap_or_else(|_| "OFF".to_string())
        .to_uppercase() == "ON";

    let ssl_cert = env::var("SSL_CERT").unwrap_or_else(|_| "ssl/server.pem".to_string());
    let ssl_key = env::var("SSL_KEY").unwrap_or_else(|_| "ssl/server.key".to_string());

    (proxy_port, target_host, target_port, ssl_enabled, ssl_cert, ssl_key)
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    dotenv().ok();
    
    let args = Args::from_args();
    let (default_proxy_port, default_target_host, default_target_port, ssl_enabled, ssl_cert, ssl_key) = load_config();

    let proxy_port = args.proxy_port.unwrap_or(default_proxy_port);
    let target_host = args.target_host.unwrap_or(default_target_host);
    let target_port = args.target_port.unwrap_or(default_target_port);

    info!("🔍 Testing connection to upstream {}:{}...", target_host, target_port);
    match std::net::TcpStream::connect(format!("{}:{}", target_host, target_port)) {
        Ok(_) => info!("✅ Upstream connection test successful"),
        Err(e) => {
            panic!("❌ Cannot connect to upstream {}:{}: {}. Make sure the target service is running.", target_host, target_port, e);
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

    let proxy = MyProxy::new(target_host.clone(), target_port, ssl_enabled);
    let mut proxy_service = http_proxy_service(&my_server.configuration, proxy);
    
    if ssl_enabled {
        if !std::path::Path::new(&ssl_cert).exists() {
            panic!("SSL certificate file not found: {}. Generate it with: openssl req -new -x509 -sha256 -key {} -out {} -days 365 -subj \"/CN=localhost\"", ssl_cert, ssl_key, ssl_cert);
        }
        if !std::path::Path::new(&ssl_key).exists() {
            panic!("SSL private key file not found: {}. Generate it with: openssl genrsa -out {} 2048", ssl_key, ssl_key);
        }

        info!("🔒 SSL/TLS enabled");
        info!("   Certificate: {}", ssl_cert);
        info!("   Private Key: {}", ssl_key);
        
        match proxy_service.add_tls(&format!("0.0.0.0:{}", proxy_port), &ssl_cert, &ssl_key) {
            Ok(_) => info!("✅ SSL listener configured successfully"),
            Err(e) => {
                info!("⚠️ SSL configuration failed, falling back to HTTP: {}", e);
                proxy_service.add_tcp(&format!("0.0.0.0:{}", proxy_port));
            }
        }
        
    } else {
        info!("🔓 SSL/TLS disabled - using HTTP");
        proxy_service.add_tcp(&format!("0.0.0.0:{}", proxy_port));
    }
    
    my_server.add_service(proxy_service);

    info!("🚀 Starting Pingora Proxy Server");
    if ssl_enabled {
        info!("📡 Listening on: https://0.0.0.0:{}", proxy_port);
    } else {
        info!("📡 Listening on: http://0.0.0.0:{}", proxy_port);
    }
    info!("🎯 Forwarding to: {}:{}", target_host, target_port);
    info!("📋 Environment variables:");
    info!("   - PROXY_PORT={}", proxy_port);
    info!("   - TARGET_HOST={}", target_host); 
    info!("   - TARGET_PORT={}", target_port);
    info!("   - SSL={}", if ssl_enabled { "ON" } else { "OFF" });
    
    if ssl_enabled {
        info!("   - SSL_CERT={}", ssl_cert);
        info!("   - SSL_KEY={}", ssl_key);
    }

    info!("🔄 Server will run indefinitely. Press Ctrl+C to stop.");
    
    my_server.run_forever();
}