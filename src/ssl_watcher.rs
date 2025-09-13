use log::info;
use openssl::x509::X509;
use std::fs;
use chrono::{DateTime, NaiveDateTime, Utc};

pub struct SslWatcher {
    pub is_good: bool,
    pub day_left: i32,
    pub error: String,
}

pub fn check_cert() -> SslWatcher {
    let cert_pem = match fs::read("ssl/server.pem") {
        Ok(data) => data,
        Err(e) => {
            return SslWatcher {
                is_good: false,
                day_left: -1,
                error: format!("Failed to read cert: {}", e),
            };
        }
    };

    let cert = match X509::from_pem(&cert_pem) {
        Ok(c) => c,
        Err(e) => {
            return SslWatcher {
                is_good: false,
                day_left: -1,
                error: format!("Failed to parse cert: {}", e),
            };
        }
    };

    let not_after_str = cert.not_after().to_string();

    info!("Certificate expiration : {}", not_after_str);

    let formats = [
        "%b %e %H:%M:%S %Y GMT",
        "%b %d %H:%M:%S %Y GMT",
        "%Y%m%d%H%M%SZ",
        "%Y-%m-%d %H:%M:%S",
    ];

    let mut not_after_datetime = None;
    
    if not_after_str.ends_with(" GMT") {
        let date_part = &not_after_str[..not_after_str.len() - 4];
        let naive_formats = [
            "%b %e %H:%M:%S %Y",
            "%b %d %H:%M:%S %Y",
        ];
        
        for format in &naive_formats {
            if let Ok(naive_dt) = NaiveDateTime::parse_from_str(date_part, format) {
                not_after_datetime = Some(DateTime::from_naive_utc_and_offset(naive_dt, Utc));
                break;
            }
        }
    }
    
    if not_after_datetime.is_none() {
        for format in &formats {
            if let Ok(dt) = DateTime::parse_from_str(&not_after_str, format) {
                not_after_datetime = Some(dt.with_timezone(&Utc));
                break;
            }
        }
    }

    let not_after_datetime = match not_after_datetime {
        Some(dt) => dt,
        None => {
            return SslWatcher {
                is_good: false,
                day_left: -1,
                error: format!("Failed to parse not_after time from string: {}", not_after_str),
            };
        }
    };

    let now = Utc::now();

    let duration = not_after_datetime - now;
    let days_left = duration.num_days() as i32;

    SslWatcher {
        is_good: true,
        day_left: days_left,
        error: String::new(),
    }
}