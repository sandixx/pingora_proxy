use async_trait::async_trait;
use log::{info, error};
use pingora_core::upstreams::peer::HttpPeer;
use pingora_core::Result;
use pingora_http::{ResponseHeader, RequestHeader};
use pingora_proxy::{ProxyHttp, Session};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::backend::Backend;
use crate::load_balancer::{LoadBalancer, LoadBalanceStrategy};

pub struct MyProxy {
    pub backends: Arc<RwLock<Vec<Backend>>>,
    pub load_balancer: Arc<LoadBalancer>,
    pub ssl_enabled: bool,
    pub custom_headers: HashMap<String, String>,
    pub remove_headers: Vec<String>,
    pub sticky_cookie_name: String,
    pub sticky_session_ttl: u64,
}

impl MyProxy {
    fn get_session_id(&self, req_header: &RequestHeader) -> Option<String> {
        if let Some(cookie_header) = req_header.headers.get("Cookie") {
            if let Ok(cookie_str) = cookie_header.to_str() {
                for cookie in cookie_str.split(';') {
                    let cookie = cookie.trim();
                    if let Some((name, value)) = cookie.split_once('=') {
                        if name.trim() == self.sticky_cookie_name {
                            return Some(value.trim().to_string());
                        }
                    }
                }
            }
        }
        None
    }
}

#[async_trait]
impl ProxyHttp for MyProxy {
    type CTX = Option<String>;

    fn new_ctx(&self) -> Self::CTX {
        None
    }

    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
        // Check for existing session cookie
        let existing_session_id = self.get_session_id(session.req_header());
        
        // If sticky sessions enabled and no session ID, generate one
        if self.load_balancer.strategy == LoadBalanceStrategy::StickySession && existing_session_id.is_none() {
            *ctx = Some(LoadBalancer::generate_session_id());
        }
        
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

    async fn upstream_peer(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<Box<HttpPeer>> {
        let backends = self.backends.read().await;
        
        // Determine session ID for sticky sessions
        let session_id = if self.load_balancer.strategy == LoadBalanceStrategy::StickySession {
            self.get_session_id(session.req_header()).or_else(|| ctx.clone())
        } else {
            None
        };

        let backend = self.load_balancer.select_backend(&backends, session_id.as_deref());
        
        match backend {
            Some(backend) => {
                let peer = Box::new(HttpPeer::new(
                    format!("{}:{}", backend.host, backend.port),
                    false,
                    "".to_string(),
                ));
                Ok(peer)
            }
            None => {
                error!("🚨 No backends available for routing");
                Err(pingora_core::Error::new_str("No backends available"))
            }
        }
    }

    async fn response_filter(&self, _session: &mut Session, upstream_response: &mut ResponseHeader, ctx: &mut Self::CTX, ) -> Result<()> {
        // Set session cookie if we generated a new session ID
        if let Some(session_id) = ctx.take() {
            // Format expiry timestamp (for Expires=)
            use chrono::{Utc, Duration};
            let expire_time = Utc::now() + Duration::seconds(self.sticky_session_ttl as i64);
            let expires_str = expire_time.format("%a, %d %b %Y %H:%M:%S GMT").to_string();

            let cookie_value = format!(
                "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}; Expires={}",
                self.sticky_cookie_name,
                session_id,
                self.sticky_session_ttl,
                expires_str
            );

            upstream_response.insert_header("Set-Cookie", cookie_value)?;
        }

        for key in &self.remove_headers {
            upstream_response.remove_header(key.as_str());
        }

        for (key, value) in &self.custom_headers {
            upstream_response.insert_header(key.clone(), value.clone())?;
        }

        Ok(())
    }
}