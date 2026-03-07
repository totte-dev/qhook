use anyhow::Result;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

pub fn verify_signature(
    provider: &str,
    secret: &str,
    payload: &[u8],
    headers: &axum::http::HeaderMap,
) -> Result<bool> {
    match provider {
        "stripe" => verify_stripe(secret, payload, headers),
        "github" => verify_github(secret, payload, headers),
        "shopify" => verify_shopify(secret, payload, headers),
        "hmac" => verify_custom_hmac(secret, payload, headers),
        _ => anyhow::bail!("Unknown verification provider: {provider}"),
    }
}

fn verify_stripe(
    secret: &str,
    payload: &[u8],
    headers: &axum::http::HeaderMap,
) -> Result<bool> {
    let sig_header = headers
        .get("Stripe-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Parse t=...,v1=... format
    let mut timestamp = "";
    let mut signature = "";
    for part in sig_header.split(',') {
        let part = part.trim();
        if let Some(t) = part.strip_prefix("t=") {
            timestamp = t;
        } else if let Some(v) = part.strip_prefix("v1=") {
            signature = v;
        }
    }

    if timestamp.is_empty() || signature.is_empty() {
        return Ok(false);
    }

    // Stripe signs: timestamp.payload
    let signed_payload = format!("{timestamp}.{}", String::from_utf8_lossy(payload));
    let expected = compute_hmac_sha256_hex(secret.as_bytes(), signed_payload.as_bytes());

    Ok(constant_time_eq(&expected, signature))
}

fn verify_github(
    secret: &str,
    payload: &[u8],
    headers: &axum::http::HeaderMap,
) -> Result<bool> {
    let sig_header = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let signature = sig_header.strip_prefix("sha256=").unwrap_or("");
    if signature.is_empty() {
        return Ok(false);
    }

    let expected = compute_hmac_sha256_hex(secret.as_bytes(), payload);
    Ok(constant_time_eq(&expected, signature))
}

fn verify_shopify(
    secret: &str,
    payload: &[u8],
    headers: &axum::http::HeaderMap,
) -> Result<bool> {
    let sig_header = headers
        .get("X-Shopify-Hmac-SHA256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if sig_header.is_empty() {
        return Ok(false);
    }

    let expected = compute_hmac_sha256_base64(secret.as_bytes(), payload);
    Ok(constant_time_eq(&expected, sig_header))
}

fn verify_custom_hmac(
    secret: &str,
    payload: &[u8],
    headers: &axum::http::HeaderMap,
) -> Result<bool> {
    // Custom HMAC: check X-Webhook-Signature header
    let sig_header = headers
        .get("X-Webhook-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if sig_header.is_empty() {
        return Ok(false);
    }

    let expected = compute_hmac_sha256_hex(secret.as_bytes(), payload);
    Ok(constant_time_eq(&expected, sig_header))
}

fn compute_hmac_sha256_hex(key: &[u8], data: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key length");
    mac.update(data);
    hex::encode(mac.finalize().into_bytes())
}

fn compute_hmac_sha256_base64(key: &[u8], data: &[u8]) -> String {
    use base64::Engine;
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key length");
    mac.update(data);
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.as_bytes().ct_eq(b.as_bytes()).into()
}
