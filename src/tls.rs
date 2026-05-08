use crate::config::TlsConfig;
use crate::error::GatewayError;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig as RustlsServerConfig;
use std::fs;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;
use tracing::{info, error};

pub fn create_tls_acceptor(config: &TlsConfig) -> Result<TlsAcceptor, GatewayError> {
    if !config.enabled {
        return Err(GatewayError::Tls("TLS not enabled".to_string()));
    }

    let certs = load_certs(&config.cert_path)?;
    let key = load_key(&config.key_path)?;

    let mut server_config = RustlsServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| GatewayError::Tls(format!("Failed to build TLS config: {}", e)))?;

    // Configure ALPN for HTTP/2 and HTTP/1.1
    server_config.alpn_protocols = vec![
        b"h2".to_vec(),
        b"http/1.1".to_vec(),
    ];

    // Optional: Configure client authentication
    if let Some(ref client_auth) = config.client_auth {
        if client_auth.enabled {
            let ca_certs = load_certs(&client_auth.ca_path)?;
            let mut root_store = rustls::RootCertStore::empty();
            for cert in ca_certs {
                root_store.add(cert).map_err(|e| {
                    GatewayError::Tls(format!("Failed to add CA cert: {}", e))
                })?;
            }

            let client_verifier = rustls::server::WebPkiClientVerifier::builder(
                Arc::new(root_store)
            )
            .build()
            .map_err(|e| GatewayError::Tls(format!("Failed to build client verifier: {}", e)))?;

            server_config = RustlsServerConfig::builder()
                .with_client_cert_verifier(client_verifier)
                .with_single_cert(
                    load_certs(&config.cert_path)?,
                    load_key(&config.key_path)?,
                )
                .map_err(|e| GatewayError::Tls(format!("Failed to rebuild TLS config: {}", e)))?;
        }
    }

    info!("TLS acceptor created successfully");
    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, GatewayError> {
    let file = fs::File::open(path)
        .map_err(|e| GatewayError::Tls(format!("Failed to open cert file: {}", e)))?;
    let mut reader = BufReader::new(file);

    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| GatewayError::Tls(format!("Failed to parse certs: {}", e)))?;

    if certs.is_empty() {
        return Err(GatewayError::Tls("No certificates found".to_string()));
    }

    Ok(certs)
}

fn load_key(path: &Path) -> Result<PrivateKeyDer<'static>, GatewayError> {
    let file = fs::File::open(path)
        .map_err(|e| GatewayError::Tls(format!("Failed to open key file: {}", e)))?;
    let mut reader = BufReader::new(file);

    // Try PKCS8 first
    if let Some(key) = rustls_pemfile::pkcs8_private_keys(&mut reader)
        .next()
        .transpose()
        .map_err(|e| GatewayError::Tls(format!("Failed to parse PKCS8 key: {}", e)))?
    {
        return Ok(PrivateKeyDer::try_from(key).unwrap());
    }

    // Reset and try RSA
    let file = fs::File::open(path)
        .map_err(|e| GatewayError::Tls(format!("Failed to reopen key file: {}", e)))?;
    let mut reader = BufReader::new(file);

    if let Some(key) = rustls_pemfile::rsa_private_keys(&mut reader)
        .next()
        .transpose()
        .map_err(|e| GatewayError::Tls(format!("Failed to parse RSA key: {}", e)))?
    {
        return Ok(PrivateKeyDer::try_from(key).unwrap());
    }

    Err(GatewayError::Tls("No valid private key found".to_string()))
}

pub fn generate_self_signed_cert() -> Result<(Vec<u8>, Vec<u8>), GatewayError> {
    // Note: In production, use real certificates from Let's Encrypt or other CA
    // This is a placeholder for development/testing
    Err(GatewayError::Tls(
        "Self-signed certificate generation not implemented. Use openssl to generate certs:\n\
         openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem -days 365 -nodes".to_string()
    ))
}
