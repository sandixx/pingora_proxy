use std::sync::{Arc, RwLock};
use std::time::Duration;
use log::{info, warn};
use reqwest::Client;
use crate::backend::Backend;
use crate::config::HealthCheckConfig;

pub struct HealthChecker;

impl HealthChecker {
    pub async fn health_check_loop(
        backends: Arc<RwLock<Vec<Backend>>>,
        config: HealthCheckConfig,
    ) {
        if !config.enabled {
            info!("🩺 Health check service is disabled");
            return;
        }
        
        let client = Client::new();
        let mut interval = tokio::time::interval(Duration::from_secs(config.interval_secs));
        
        info!("🩺 Starting health check service (interval: {}s)", config.interval_secs);
        
        loop {
            interval.tick().await;
            
            let mut backends_write = backends.write().unwrap();
            for backend in backends_write.iter_mut() {
                match HealthChecker::check_backend(&client, backend, &config).await {
                    Ok(healthy) => {
                        backend.healthy = healthy;
                        backend.last_checked = Some(std::time::Instant::now());
                    }
                    Err(e) => {
                        warn!("Health check failed for {}:{}: {}", backend.host, backend.port, e);
                        backend.healthy = false;
                    }
                }
            }
        }
    }
    
    async fn check_backend(
        client: &Client,
        backend: &Backend,
        config: &HealthCheckConfig,
    ) -> Result<bool, reqwest::Error> {
        let url = format!("http://{}:{}{}", backend.host, backend.port, config.path);
        let response = client
            .get(&url)
            .timeout(Duration::from_secs(config.timeout_secs))
            .send()
            .await?;
        
        Ok(config.success_codes.contains(&response.status().as_u16()))
    }
}