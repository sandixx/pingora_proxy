use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use std::sync::Arc;
use log::{info, warn};
use reqwest::Client;
use crate::backend::Backend;

#[derive(Clone)]
pub struct HealthCheckConfig {
    pub enabled: bool,
    pub path: String,
    pub interval_secs: u64,
    pub timeout_secs: u64,
    pub success_codes: Vec<u16>,
}

pub struct HealthChecker;

impl HealthChecker {
    pub async fn health_check_loop(backends: Arc<RwLock<Vec<Backend>>>, config: HealthCheckConfig) {
        if !config.enabled {
            info!("🩺 Health checks disabled");
            return;
        }

        info!("🩺 Starting health check service (interval: {}s)", config.interval_secs);
        
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .unwrap();

        loop {
            tokio::time::sleep(Duration::from_secs(config.interval_secs)).await;
            
            let mut backends_write = backends.write().await;
            for backend in backends_write.iter_mut() {
                // Direct URL construction - Docker handles the resolution
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
                            warn!("❌ Backend url: {} health check failed: {}", url, e);
                            backend.healthy = false;
                        }
                        backend.last_checked = Some(Instant::now());
                    }
                }
            }
        }
    }
}