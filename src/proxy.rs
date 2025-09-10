use async_trait::async_trait;
use log::{info, warn, error};
use pingora_core::upstreams::peer::HttpPeer;
use pingora_core::Result;
use pingora_http::ResponseHeader;
use pingora_proxy::{ProxyHttp, Session};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::backend::Backend;

pub struct MyProxy {
    pub backends: Arc<RwLock<Vec<Backend>>>,
    pub ssl_enabled: bool,
    pub custom_headers: HashMap<String, String>,
    pub remove_headers: Vec<String>,
    pub counter: AtomicUsize,
    pub round_robin: bool, // Add this field
}

impl MyProxy {
    pub async fn select_backend(&self) -> Option<Backend> {
        let backends = self.backends.read().await;
        let healthy_backends: Vec<&Backend> = backends.iter().filter(|b| b.healthy).collect();
        
        if healthy_backends.is_empty() {
            warn!("⚠️ No healthy backends available, falling back to all backends");
            // Fallback to all backends if none are healthy
            if self.round_robin {
                // Simple round-robin for fallback
                let index = self.counter.fetch_add(1, Ordering::Relaxed) % backends.len();
                return backends.get(index).cloned();
            } else {
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
        }

        if self.round_robin {
            // Simple round-robin selection from healthy backends
            let index = self.counter.fetch_add(1, Ordering::Relaxed) % healthy_backends.len();
            return Some(healthy_backends[index].clone());
        } else {
            // Weighted selection
            let total_weight: usize = healthy_backends.iter().map(|b| b.weight).sum();

            if total_weight == 0 {
                return healthy_backends.first().cloned().cloned();
            }

            let choice = (self.counter.fetch_add(1, Ordering::Relaxed) % 100) as usize;
            let mut acc = 0;
            
            for b in &healthy_backends {
                acc += b.weight;
                if choice < acc {
                    return Some((*b).clone());
                }
            }
            
            healthy_backends.first().cloned().cloned()
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

    async fn upstream_peer(&self, _session: &mut Session, _ctx: &mut Self::CTX) -> Result<Box<HttpPeer>> {
        match self.select_backend().await {
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

    async fn response_filter(
        &self,
        _session: &mut Session,
        upstream_response: &mut ResponseHeader,
        _ctx: &mut Self::CTX,
    ) -> Result<()> {
        for key in &self.remove_headers {
            upstream_response.remove_header(key.as_str());
        }

        for (key, value) in &self.custom_headers {
            upstream_response.insert_header(key.clone(), value.clone())?;
        }

        Ok(())
    }
}