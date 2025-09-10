use std::env;
use std::collections::HashMap;
use log::{self, warn};

use crate::backend::Backend;
use crate::health_check::HealthCheckConfig;
use crate::load_balancer::LoadBalanceStrategy;

pub fn load_balance_strategy() -> LoadBalanceStrategy {
    let strategy_str = env::var("LOAD_BALANCE_STRATEGY")
        .unwrap_or_else(|_| "weighted".to_string())
        .to_lowercase();
    
    match LoadBalanceStrategy::from_str(&strategy_str) {
        Some(strategy) => strategy,
        None => {
            warn!("⚠️ Unknown load balance strategy '{}', defaulting to 'weighted'", strategy_str);
            LoadBalanceStrategy::Weighted
        }
    }
}

pub fn load_sticky_cookie_name() -> String {
    env::var("STICKY_COOKIE_NAME")
        .unwrap_or_else(|_| "PINGORA_SESSION".to_string())
}

pub fn load_sticky_session_ttl() -> u64 {
    std::env::var("STICKY_SESSION_TTL")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(3600) // default 1 hour
}


pub fn load_backends() -> Vec<Backend> {
    let mut backends = Vec::new();
    
    if let Ok(val) = env::var("BACKENDS") {
        for entry in val.split(',') {
            let parts: Vec<&str> = entry.split(':').collect();
            
            if parts.len() >= 2 {
                if let Ok(port) = parts[1].parse::<u16>() {
                    let weight = if parts.len() == 3 {
                        parts[2].parse::<usize>().unwrap_or(1)
                    } else {
                        1 // Default weight
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
    
    // Normalize weights to sum to 100
    let total: usize = backends.iter().map(|b| b.weight).sum();
    if total != 100 {
        let factor = 100.0 / (total as f64);
        for b in backends.iter_mut() {
            b.weight = ((b.weight as f64) * factor).round() as usize;
        }
    }
    
    backends
}

pub fn load_health_check_config() -> HealthCheckConfig {
    let enabled = env::var("HEALTH_CHECK_ENABLED").unwrap_or_else(|_| "true".to_string()).to_lowercase() == "true";
    let path = env::var("HEALTH_CHECK_PATH").unwrap_or_else(|_| "/health".to_string());
    let interval_secs: u64 = env::var("HEALTH_CHECK_INTERVAL").unwrap_or_else(|_| "30".to_string()).parse::<u64>().expect("HEALTH_CHECK_INTERVAL must be a valid u64 number");
    let timeout_secs = env::var("HEALTH_CHECK_TIMEOUT").unwrap_or_else(|_| "5".to_string()).parse().unwrap_or(5);
    let success_codes_str = env::var("HEALTH_CHECK_SUCCESS_CODES").unwrap_or_else(|_| "200".to_string());
    let success_codes: Vec<u16> = success_codes_str.split(',').filter_map(|s| s.trim().parse().ok()).collect();

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
        let trimmed_val = val.trim_matches('"');
        match serde_json::from_str::<HashMap<String, String>>(trimmed_val) {
            Ok(map) => headers = map,
            Err(e) => {
                log::warn!("⚠️ Failed to parse CUSTOM_HEADER env: {} (value={})", e, val);
                if let Some(colon_pos) = trimmed_val.find(':') {
                    let key = trimmed_val[..colon_pos].trim().to_string();
                    let value = trimmed_val[colon_pos+1..].trim().to_string();
                    headers.insert(key, value);
                }
            }
        }
    }
    headers
}

pub fn load_remove_headers() -> Vec<String> {
    if let Ok(val) = env::var("REMOVE_HEADER") {
        let trimmed_val = val.trim_matches('"');
        match serde_json::from_str::<Vec<String>>(trimmed_val) {
            Ok(list) => list,
            Err(e) => {
                log::warn!("⚠️ Failed to parse REMOVE_HEADER env: {} (value={})", e, val);
                trimmed_val.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
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