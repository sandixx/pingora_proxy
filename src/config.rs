use std::env;
use std::collections::HashMap;
use log;

use crate::backend::Backend;
use crate::health_check::HealthCheckConfig;

pub fn load_backends() -> Vec<Backend> {
    let mut backends = Vec::new();
    if let Ok(val) = env::var("BACKENDS") {
        for entry in val.split(',') {
            let parts: Vec<&str> = entry.split(':').collect();
            
            // Handle different formats: host:port or host:port:weight
            if parts.len() >= 2 {
                if let Ok(port) = parts[1].parse::<u16>() {
                    let weight = if parts.len() == 3 {
                        parts[2].parse::<usize>().unwrap_or(100)
                    } else {
                        100 // Default weight if not specified
                    };
                    
                    backends.push(Backend {
                        host: parts[0].to_string(),
                        port,
                        weight,
                        healthy: true,
                        last_checked: None,
                    });
                }
            }
        }
    }
    
    if backends.is_empty() {
        panic!("❌ BACKENDS must be set and not empty!");
    }

    // If there's only one backend, set its weight to 100
    if backends.len() == 1 {
        backends[0].weight = 100;
    } else {
        // Only normalize weights if there are multiple backends
        let total: usize = backends.iter().map(|b| b.weight).sum();
        if total != 100 {
            log::warn!("⚠️ BACKENDS weights sum to {} instead of 100 (auto-normalizing)", total);
            let factor = 100.0 / (total as f64);
            for b in backends.iter_mut() {
                b.weight = ((b.weight as f64) * factor).round() as usize;
            }
        }
    }

    backends
}

pub fn load_health_check_config() -> HealthCheckConfig {
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

pub fn load_custom_headers() -> HashMap<String, String> {
    let mut headers = HashMap::new();
    if let Ok(val) = env::var("CUSTOM_HEADER") {
        match serde_json::from_str::<HashMap<String, String>>(&val) {
            Ok(map) => headers = map,
            Err(e) => {
                log::warn!("⚠️ Failed to parse CUSTOM_HEADER env: {} (value={})", e, val);
            }
        }
    }
    headers
}

pub fn load_remove_headers() -> Vec<String> {
    if let Ok(val) = env::var("REMOVE_HEADER") {
        match serde_json::from_str::<Vec<String>>(&val) {
            Ok(list) => list,
            Err(e) => {
                log::warn!("⚠️ Failed to parse REMOVE_HEADER env: {} (value={})", e, val);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    }
}

pub fn get_proxy_port(args_proxy_port: Option<u16>) -> u16 {
    args_proxy_port.unwrap_or_else(|| {
        env::var("PROXY_PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse()
            .unwrap()
    })
}

pub fn is_ssl_enabled() -> bool {
    env::var("SSL")
        .unwrap_or_else(|_| "OFF".to_string())
        .to_uppercase() == "ON"
}

pub fn get_ssl_cert() -> String {
    env::var("SSL_CERT").unwrap_or_else(|_| "ssl/server.pem".to_string())
}

pub fn get_ssl_key() -> String {
    env::var("SSL_KEY").unwrap_or_else(|_| "ssl/server.key".to_string())
}