use rcgen::{Certificate, CertificateParams, DistinguishedName, DnType, SanType};
use std::fs;
use time::{OffsetDateTime, Duration};

pub struct GenerateSslStatus {
    pub status: String,
    pub error: String,
}

pub fn generate_cert() -> GenerateSslStatus {
    if let Err(e) = fs::create_dir_all("ssl") {
        return GenerateSslStatus {
            status: "Error".to_string(),
            error: format!("Failed to create ssl directory: {}", e),
        };
    }

    // Get current time and add 365 days
    let now = OffsetDateTime::now_utc();
    let one_year_later = now + Duration::days(365);

    // Configure certificate parameters
    let mut params = CertificateParams::default();
    params.not_before = now;
    params.not_after = one_year_later;
    params.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CountryName, "ID");
        dn.push(DnType::StateOrProvinceName, "NorthSumatera");
        dn.push(DnType::LocalityName, "Medan");
        dn.push(DnType::OrganizationName, "Organization");
        dn.push(DnType::CommonName, "localhost");
        dn
    };
    params.subject_alt_names = vec![SanType::DnsName("localhost".to_string())];

    // Generate the certificate
    let cert = match Certificate::from_params(params) {
        Ok(cert) => cert,
        Err(e) => {
            return GenerateSslStatus {
                status: "Error".to_string(),
                error: format!("Failed to generate certificate: {}", e),
            };
        }
    };

    // Serialize private key and certificate
    let private_key_pem = cert.serialize_private_key_pem();
    let cert_pem = cert.serialize_pem().unwrap(); // Safe as long as cert was generated

    // Write files
    if let Err(e) = fs::write("ssl/server.key", &private_key_pem) {
        return GenerateSslStatus {
            status: "Error".to_string(),
            error: format!("Failed to write private key: {}", e),
        };
    }

    if let Err(e) = fs::write("ssl/server.pem", &cert_pem) {
        return GenerateSslStatus {
            status: "Error".to_string(),
            error: format!("Failed to write certificate: {}", e),
        };
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions("ssl/server.key", fs::Permissions::from_mode(0o600)) {
            log::warn!("Failed to set permissions on private key: {}", e);
        }
    }

    GenerateSslStatus {
        status: "Success".to_string(),
        error: "".to_string(),
    }
}