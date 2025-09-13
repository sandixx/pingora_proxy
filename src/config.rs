use std::env;
use std::path::Path;
use std::process::{self, Command};
use std::collections::HashMap;
use log::{self, info, warn};

use crate::backend::Backend;
use crate::load_balancer::LoadBalanceStrategy;

#[derive(Debug, Clone)]
pub struct HealthCheckConfig {
    pub enabled: bool,
    pub path: String,
    pub interval_secs: u64,
    pub timeout_secs: u64,
    pub success_codes: Vec<u16>,
}

pub fn load_balance_strategy() -> LoadBalanceStrategy {
    let strategy_str = env::var("LOAD_BALANCE_STRATEGY")
        .unwrap_or_else(|_| "weighted".to_string())
        .to_lowercase();
    
    match strategy_str.as_str() {
        "round_robin" | "round-robin" | "roundrobin" => LoadBalanceStrategy::RoundRobin,
        "weighted" => LoadBalanceStrategy::Weighted,
        "least_connections" | "least-connections" | "leastconnections" => LoadBalanceStrategy::LeastConnections,
        "sticky_session" | "sticky-session" | "stickysession" => LoadBalanceStrategy::StickySession,
        "random" => LoadBalanceStrategy::Random,
        _ => {
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
        .unwrap_or(3600)
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

pub struct SslEnabled {
    pub status: bool,
    pub cert_loc: String,
    pub key_loc: String,
}

pub fn is_ssl_enabled() -> SslEnabled {
    let ssl = env::var("SSL").unwrap_or_else(|_| "OFF".to_string()).to_uppercase() == "ON";
    let cert_loc = "ssl/server.pem".to_string();
    let key_loc = "ssl/server.key".to_string();
    let cert = Path::new(&cert_loc);
    let key = Path::new(&key_loc);

    if ssl {
        if !cert.exists() || !key.exists() {
            let gen_ssl = generate_ssl();

            if gen_ssl.status != "Success".to_string() {
                warn!("{}", gen_ssl.error);
                process::exit(1);
            }

            info!("SSL Generated !!!");
        }
    }

    if !cert.exists() {
        warn!("SSL CERT is not exist!");
        process::exit(1);
    }

    if !key.exists() {
        warn!("SSL KEY is not exist!");
        process::exit(1);
    }

    SslEnabled {
        status: ssl,
        cert_loc: "ssl/server.pem".to_string(),
        key_loc: "ssl/server.key".to_string(),
    }
}

pub struct GenerateSslStatus {
    pub status: String,
    pub error: String,
}

pub fn generate_ssl() -> GenerateSslStatus {
    match Command::new("mkdir").args(&["-p", "ssl"]).status() {
        Ok(status) if status.success() => {
            info!("Created ssl directory.");
        }
        Ok(status) => {
            return GenerateSslStatus {status: "Error".to_string(), error: format!("mkdir failed with status: {}", status)};
        }
        Err(err) => {
            return GenerateSslStatus {status: "Error".to_string(), error: format!("Failed to create ssl directory: {}", err)};
        }
    }

    match Command::new("openssl")
        .args(&[
            "req", "-x509", "-newkey", "rsa:2048",
            "-keyout", "ssl/server.key",
            "-out", "ssl/server.pem",
            "-days", "365",
            "-nodes",
            "-subj", "/C=ID/ST=NorthSumatera/L=Medan/O=Organization/CN=localhost",
        ])
        .status()
    {
        Ok(status) if status.success() => {
            info!("Generated private key and certificate.");
        }
        Ok(status) => {
            return GenerateSslStatus {status: "Error".to_string(), error: format!("openssl req failed with status: {}", status)};
        }
        Err(err) => {
            return GenerateSslStatus {status: "Error".to_string(), error: format!("Failed to run openssl req: {}", err)};
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions("ssl/server.key", std::fs::Permissions::from_mode(0o600)) {
            warn!("Failed to set permissions on private key: {}", e);
        }
    }

    if let Err(e) = verify_ssl_files() {
        return GenerateSslStatus {status: "Error".to_string(), error: format!("Generated SSL files are invalid: {}", e)};
    }

    GenerateSslStatus {status: "Success".to_string(), error: "".to_string()}
}

fn verify_ssl_files() -> Result<(), String> {
    let key_output = Command::new("openssl")
        .args(&["rsa", "-in", "ssl/server.key", "-check", "-noout"])
        .output()
        .map_err(|e| format!("Failed to verify private key: {}", e))?;
    
    if !key_output.status.success() {
        return Err(format!("Private key is invalid: {}", String::from_utf8_lossy(&key_output.stderr)));
    }

    let cert_output = Command::new("openssl")
        .args(&["x509", "-in", "ssl/server.pem", "-noout"])
        .output()
        .map_err(|e| format!("Failed to verify certificate: {}", e))?;
    
    if !cert_output.status.success() {
        return Err(format!("Certificate is invalid: {}", String::from_utf8_lossy(&cert_output.stderr)));
    }

    Ok(())
}