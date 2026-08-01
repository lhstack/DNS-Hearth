//! X.509 certificate inspection.
//!
//! Pure technical helper: parses a PEM chain and reports validity. Carries no
//! domain meaning, so it lives in `common` rather than in a business module.

use chrono::{DateTime, Utc};
use x509_parser::prelude::*;

use super::error::{AppError, AppResult};

/// Details extracted from the leaf certificate of a PEM chain.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CertificateDetails {
    pub subject: String,
    pub issuer: String,
    pub not_before: String,
    pub not_after: String,
    pub serial_number: String,
    pub is_expired: bool,
    pub days_until_expiry: i64,
}

/// Parse the leaf certificate of a PEM chain and report its validity window.
pub fn inspect_pem_chain(cert_pem: &str) -> AppResult<CertificateDetails> {
    let der_chain: Vec<_> = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .filter_map(|entry| entry.ok())
        .collect();

    let leaf = der_chain
        .first()
        .ok_or_else(|| AppError::Validation("无法解析证书内容".to_string()))?;

    let (_, cert) = X509Certificate::from_der(leaf)
        .map_err(|e| AppError::Validation(format!("证书解析失败: {}", e)))?;

    let not_before = cert.validity().not_before.to_datetime();
    let not_after = cert.validity().not_after.to_datetime();

    let expires_at = DateTime::from_timestamp(not_after.unix_timestamp(), 0)
        .ok_or_else(|| AppError::Validation("证书有效期时间戳无效".to_string()))?;
    let now = Utc::now();

    Ok(CertificateDetails {
        subject: cert.subject().to_string(),
        issuer: cert.issuer().to_string(),
        not_before: not_before.to_string(),
        not_after: not_after.to_string(),
        serial_number: cert.serial.to_str_radix(16),
        is_expired: now > expires_at,
        days_until_expiry: (expires_at - now).num_days(),
    })
}
