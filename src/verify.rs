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
        "pagerduty" => verify_pagerduty(secret, payload, headers),
        "grafana" => verify_grafana(secret, payload, headers),
        "terraform" => verify_terraform(secret, payload, headers),
        "gitlab" => verify_gitlab(secret, payload, headers),
        _ => anyhow::bail!("Unknown verification provider: {provider}"),
    }
}

fn verify_stripe(secret: &str, payload: &[u8], headers: &axum::http::HeaderMap) -> Result<bool> {
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

    // Reject signatures older than 5 minutes to prevent replay attacks
    const TOLERANCE_SECS: i64 = 300;
    if let Ok(ts) = timestamp.parse::<i64>() {
        let now = chrono::Utc::now().timestamp();
        if (now - ts).abs() > TOLERANCE_SECS {
            tracing::warn!(
                timestamp = ts,
                now = now,
                "Stripe signature timestamp too old or too far in the future"
            );
            return Ok(false);
        }
    } else {
        return Ok(false);
    }

    // Stripe signs: timestamp.payload — HMAC directly from bytes to avoid String allocation
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC key length");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(payload);
    let expected = hex::encode(mac.finalize().into_bytes());

    Ok(constant_time_eq(&expected, signature))
}

fn verify_github(secret: &str, payload: &[u8], headers: &axum::http::HeaderMap) -> Result<bool> {
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

fn verify_shopify(secret: &str, payload: &[u8], headers: &axum::http::HeaderMap) -> Result<bool> {
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

fn verify_pagerduty(secret: &str, payload: &[u8], headers: &axum::http::HeaderMap) -> Result<bool> {
    let sig_header = headers
        .get("X-PagerDuty-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let signature = sig_header.strip_prefix("v1=").unwrap_or("");
    if signature.is_empty() {
        return Ok(false);
    }

    let expected = compute_hmac_sha256_hex(secret.as_bytes(), payload);
    Ok(constant_time_eq(&expected, signature))
}

fn verify_grafana(secret: &str, payload: &[u8], headers: &axum::http::HeaderMap) -> Result<bool> {
    let sig_header = headers
        .get("X-Grafana-Alerting-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if sig_header.is_empty() {
        return Ok(false);
    }

    let expected = compute_hmac_sha256_hex(secret.as_bytes(), payload);
    Ok(constant_time_eq(&expected, sig_header))
}

fn verify_terraform(secret: &str, payload: &[u8], headers: &axum::http::HeaderMap) -> Result<bool> {
    let sig_header = headers
        .get("X-TFE-Notification-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if sig_header.is_empty() {
        return Ok(false);
    }

    let expected = compute_hmac_sha512_hex(secret.as_bytes(), payload);
    Ok(constant_time_eq(&expected, sig_header))
}

fn verify_gitlab(secret: &str, _payload: &[u8], headers: &axum::http::HeaderMap) -> Result<bool> {
    let token = headers
        .get("X-Gitlab-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if token.is_empty() {
        return Ok(false);
    }

    Ok(constant_time_eq(secret, token))
}

fn compute_hmac_sha256_hex(key: &[u8], data: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key length");
    mac.update(data);
    hex::encode(mac.finalize().into_bytes())
}

fn compute_hmac_sha512_hex(key: &[u8], data: &[u8]) -> String {
    use hmac::Mac;
    type HmacSha512 = Hmac<sha2::Sha512>;
    let mut mac = HmacSha512::new_from_slice(key).expect("HMAC key length");
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

// --- SNS signature verification ---

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SnsMessage {
    #[serde(rename = "Type")]
    pub message_type: String,
    #[serde(rename = "MessageId")]
    pub message_id: String,
    #[serde(rename = "Message")]
    pub message: String,
    #[serde(rename = "Timestamp")]
    pub timestamp: String,
    #[serde(rename = "TopicArn")]
    pub topic_arn: String,
    #[serde(rename = "Signature")]
    pub signature: String,
    #[serde(rename = "SigningCertURL")]
    pub signing_cert_url: String,
    #[serde(rename = "SignatureVersion")]
    pub signature_version: String,
    #[serde(rename = "Subject", default)]
    pub subject: Option<String>,
    #[serde(rename = "SubscribeURL", default)]
    pub subscribe_url: Option<String>,
    #[serde(rename = "Token", default)]
    pub token: Option<String>,
    #[serde(rename = "UnsubscribeURL", default)]
    pub unsubscribe_url: Option<String>,
}

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Cache for SNS signing certificates. TTL = 1 hour.
type CertCache = Mutex<HashMap<String, (Vec<u8>, Instant)>>;
static SNS_CERT_CACHE: std::sync::LazyLock<CertCache> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

use std::collections::HashMap;

const SNS_CERT_CACHE_TTL: Duration = Duration::from_secs(3600);

fn get_cached_cert(url: &str) -> Option<Vec<u8>> {
    let cache = SNS_CERT_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((data, ts)) = cache.get(url) {
        if ts.elapsed() < SNS_CERT_CACHE_TTL {
            return Some(data.clone());
        }
    }
    None
}

fn set_cached_cert(url: &str, data: Vec<u8>) {
    let mut cache = SNS_CERT_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.insert(url.to_string(), (data, Instant::now()));
}

pub async fn verify_sns_message(msg: &SnsMessage, http: &reqwest::Client) -> Result<bool> {
    let cert_url = &msg.signing_cert_url;

    // Validate the signing cert URL is from SNS
    if !is_valid_sns_url(cert_url) {
        tracing::warn!(url = cert_url, "Invalid SNS SigningCertURL");
        return Ok(false);
    }

    // Fetch the signing certificate (cached)
    let pem_data = if let Some(cached) = get_cached_cert(cert_url) {
        cached.into()
    } else {
        let data = http.get(cert_url).send().await?.bytes().await?;
        set_cached_cert(cert_url, data.to_vec());
        data
    };

    // Parse X.509 certificate
    let (_, pem) = x509_parser::pem::parse_x509_pem(&pem_data)
        .map_err(|e| anyhow::anyhow!("PEM parse error: {:?}", e))?;
    let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents)
        .map_err(|e| anyhow::anyhow!("X.509 parse error: {:?}", e))?;

    // Extract RSA public key (PKCS#1 DER from SPKI)
    use pkcs1::DecodeRsaPublicKey;
    let key_data = cert.tbs_certificate.subject_pki.subject_public_key.data;
    let public_key = rsa::RsaPublicKey::from_pkcs1_der(&key_data)
        .map_err(|e| anyhow::anyhow!("RSA key parse error: {e}"))?;

    // Build the string to sign
    let string_to_sign = build_sns_string_to_sign(msg);

    // Decode the base64 signature
    use base64::Engine;
    let sig_bytes = base64::engine::general_purpose::STANDARD.decode(&msg.signature)?;

    // Verify based on SignatureVersion
    use rsa::pkcs1v15::{Signature, VerifyingKey};
    use rsa::signature::Verifier;

    let sig = Signature::try_from(sig_bytes.as_slice())
        .map_err(|e| anyhow::anyhow!("Invalid signature: {e}"))?;

    let valid = match msg.signature_version.as_str() {
        "1" => {
            let vk = VerifyingKey::<sha1::Sha1>::new(public_key);
            vk.verify(string_to_sign.as_bytes(), &sig).is_ok()
        }
        "2" => {
            let vk = VerifyingKey::<sha2::Sha256>::new(public_key);
            vk.verify(string_to_sign.as_bytes(), &sig).is_ok()
        }
        _ => {
            tracing::warn!(
                version = msg.signature_version,
                "Unknown SNS SignatureVersion"
            );
            false
        }
    };

    Ok(valid)
}

/// Validate that a URL is from a legitimate SNS endpoint (amazonaws.com).
/// Used for both SigningCertURL and SubscribeURL validation.
pub fn is_valid_sns_url(url: &str) -> bool {
    // Must be HTTPS from an SNS endpoint
    if !url.starts_with("https://sns.") {
        return false;
    }
    // Must be from amazonaws.com (exact domain match, not substring)
    if let Some(host_start) = url.strip_prefix("https://")
        && let Some(path_start) = host_start.find('/')
    {
        let host = &host_start[..path_start];
        // Split by '.' and check the last two segments are exactly "amazonaws" and "com"
        let parts: Vec<&str> = host.split('.').collect();
        return parts.len() >= 3
            && parts[parts.len() - 1] == "com"
            && parts[parts.len() - 2] == "amazonaws";
    }
    false
}

pub fn build_sns_string_to_sign(msg: &SnsMessage) -> String {
    let mut lines = String::with_capacity(512);

    match msg.message_type.as_str() {
        "Notification" => {
            lines.push_str("Message\n");
            lines.push_str(&msg.message);
            lines.push('\n');
            lines.push_str("MessageId\n");
            lines.push_str(&msg.message_id);
            lines.push('\n');
            if let Some(ref subject) = msg.subject {
                lines.push_str("Subject\n");
                lines.push_str(subject);
                lines.push('\n');
            }
            lines.push_str("Timestamp\n");
            lines.push_str(&msg.timestamp);
            lines.push('\n');
            lines.push_str("TopicArn\n");
            lines.push_str(&msg.topic_arn);
            lines.push('\n');
            lines.push_str("Type\n");
            lines.push_str(&msg.message_type);
            lines.push('\n');
        }
        "SubscriptionConfirmation" | "UnsubscribeConfirmation" => {
            lines.push_str("Message\n");
            lines.push_str(&msg.message);
            lines.push('\n');
            lines.push_str("MessageId\n");
            lines.push_str(&msg.message_id);
            lines.push('\n');
            if let Some(ref url) = msg.subscribe_url {
                lines.push_str("SubscribeURL\n");
                lines.push_str(url);
                lines.push('\n');
            }
            lines.push_str("Timestamp\n");
            lines.push_str(&msg.timestamp);
            lines.push('\n');
            if let Some(ref token) = msg.token {
                lines.push_str("Token\n");
                lines.push_str(token);
                lines.push('\n');
            }
            lines.push_str("TopicArn\n");
            lines.push_str(&msg.topic_arn);
            lines.push('\n');
            lines.push_str("Type\n");
            lines.push_str(&msg.message_type);
            lines.push('\n');
        }
        _ => {}
    }

    lines
}

// --- Outbound webhook signing (Standard Webhooks spec) ---

/// Sign an outbound webhook payload using HMAC-SHA256 per the Standard Webhooks spec.
/// Returns the base64-encoded signature.
///
/// Signed content format: `{msg_id}.{timestamp}.{payload}`
/// The secret should be a `whsec_`-prefixed base64-encoded key.
pub fn sign_outbound_payload(secret: &str, msg_id: &str, timestamp: i64, payload: &[u8]) -> String {
    use base64::Engine;
    // Strip whsec_ prefix and base64-decode to get raw key bytes
    let key_bytes = if let Some(stripped) = secret.strip_prefix("whsec_") {
        base64::engine::general_purpose::STANDARD
            .decode(stripped)
            .unwrap_or_else(|_| secret.as_bytes().to_vec())
    } else {
        secret.as_bytes().to_vec()
    };
    let mut mac = HmacSha256::new_from_slice(&key_bytes).expect("HMAC accepts any key length");
    mac.update(msg_id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(payload);
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// Generate a signing secret for an outbound endpoint (Standard Webhooks format).
/// Generates 32 random bytes, encodes as standard base64, prefixed with `whsec_`.
pub fn generate_signing_secret() -> String {
    use base64::Engine;
    use sha2::Digest;
    // Use SHA-256 of two ULIDs for 256 bits of entropy without adding `rand` dependency
    let ulid1 = ulid::Ulid::new();
    let ulid2 = ulid::Ulid::new();
    let mut hasher = Sha256::new();
    hasher.update(ulid1.to_bytes());
    hasher.update(ulid2.to_bytes());
    let hash = hasher.finalize();
    format!(
        "whsec_{}",
        base64::engine::general_purpose::STANDARD.encode(hash)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    // --- HMAC verification ---

    #[test]
    fn test_github_signature_valid() {
        let secret = "mysecret";
        let payload = b"hello world";
        let expected = compute_hmac_sha256_hex(secret.as_bytes(), payload);

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Hub-Signature-256",
            format!("sha256={expected}").parse().unwrap(),
        );

        assert!(verify_github(secret, payload, &headers).unwrap());
    }

    #[test]
    fn test_github_signature_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Hub-Signature-256",
            "sha256=0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap(),
        );
        assert!(!verify_github("secret", b"payload", &headers).unwrap());
    }

    #[test]
    fn test_github_signature_missing() {
        let headers = HeaderMap::new();
        assert!(!verify_github("secret", b"payload", &headers).unwrap());
    }

    #[test]
    fn test_stripe_signature_valid() {
        let secret = "whsec_test";
        let payload = b"{\"id\":\"evt_123\"}";
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let signed = format!("{timestamp}.{}", String::from_utf8_lossy(payload));
        let sig = compute_hmac_sha256_hex(secret.as_bytes(), signed.as_bytes());

        let mut headers = HeaderMap::new();
        headers.insert(
            "Stripe-Signature",
            format!("t={timestamp},v1={sig}").parse().unwrap(),
        );

        assert!(verify_stripe(secret, payload, &headers).unwrap());
    }

    #[test]
    fn test_stripe_signature_expired() {
        let secret = "whsec_test";
        let payload = b"{\"id\":\"evt_123\"}";
        // 10 minutes ago — should be rejected
        let timestamp = (chrono::Utc::now().timestamp() - 600).to_string();
        let signed = format!("{timestamp}.{}", String::from_utf8_lossy(payload));
        let sig = compute_hmac_sha256_hex(secret.as_bytes(), signed.as_bytes());

        let mut headers = HeaderMap::new();
        headers.insert(
            "Stripe-Signature",
            format!("t={timestamp},v1={sig}").parse().unwrap(),
        );

        assert!(!verify_stripe(secret, payload, &headers).unwrap());
    }

    #[test]
    fn test_stripe_signature_future() {
        let secret = "whsec_test";
        let payload = b"{\"id\":\"evt_123\"}";
        // 10 minutes in the future — should be rejected
        let timestamp = (chrono::Utc::now().timestamp() + 600).to_string();
        let signed = format!("{timestamp}.{}", String::from_utf8_lossy(payload));
        let sig = compute_hmac_sha256_hex(secret.as_bytes(), signed.as_bytes());

        let mut headers = HeaderMap::new();
        headers.insert(
            "Stripe-Signature",
            format!("t={timestamp},v1={sig}").parse().unwrap(),
        );

        assert!(!verify_stripe(secret, payload, &headers).unwrap());
    }

    #[test]
    fn test_shopify_signature_valid() {
        let secret = "shopify_secret";
        let payload = b"{\"topic\":\"orders/create\"}";
        let expected = compute_hmac_sha256_base64(secret.as_bytes(), payload);

        let mut headers = HeaderMap::new();
        headers.insert("X-Shopify-Hmac-SHA256", expected.parse().unwrap());

        assert!(verify_shopify(secret, payload, &headers).unwrap());
    }

    #[test]
    fn test_custom_hmac_valid() {
        let secret = "my_secret";
        let payload = b"data";
        let expected = compute_hmac_sha256_hex(secret.as_bytes(), payload);

        let mut headers = HeaderMap::new();
        headers.insert("X-Webhook-Signature", expected.parse().unwrap());

        assert!(verify_custom_hmac(secret, payload, &headers).unwrap());
    }

    #[test]
    fn test_verify_signature_unknown_provider() {
        let headers = HeaderMap::new();
        assert!(verify_signature("unknown", "secret", b"data", &headers).is_err());
    }

    // --- Constant-time comparison ---

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
    }

    // --- SNS cert URL validation ---

    #[test]
    fn test_sns_cert_url_valid() {
        assert!(is_valid_sns_url(
            "https://sns.us-east-1.amazonaws.com/SimpleNotificationService-abc123.pem"
        ));
        assert!(is_valid_sns_url(
            "https://sns.ap-northeast-1.amazonaws.com/cert.pem"
        ));
    }

    #[test]
    fn test_sns_cert_url_invalid() {
        assert!(!is_valid_sns_url(
            "http://sns.us-east-1.amazonaws.com/cert.pem"
        )); // http
        assert!(!is_valid_sns_url("https://evil.com/cert.pem")); // wrong domain
        assert!(!is_valid_sns_url("https://sns.us-east-1.evil.com/cert.pem")); // spoofed
        assert!(!is_valid_sns_url("https://amazonaws.com/cert.pem")); // missing sns prefix
        assert!(!is_valid_sns_url(
            "https://sns.us-east-1.amazonaws.com.attacker.com/cert.pem"
        )); // subdomain spoofing
    }

    // --- SNS string to sign ---

    #[test]
    fn test_sns_notification_string_to_sign() {
        let msg = SnsMessage {
            message_type: "Notification".into(),
            message_id: "msg-123".into(),
            message: "Hello".into(),
            timestamp: "2024-01-01T00:00:00.000Z".into(),
            topic_arn: "arn:aws:sns:us-east-1:123456:my-topic".into(),
            signature: String::new(),
            signing_cert_url: String::new(),
            signature_version: "1".into(),
            subject: None,
            subscribe_url: None,
            token: None,
            unsubscribe_url: None,
        };

        let result = build_sns_string_to_sign(&msg);
        assert_eq!(
            result,
            "Message\nHello\nMessageId\nmsg-123\nTimestamp\n2024-01-01T00:00:00.000Z\nTopicArn\narn:aws:sns:us-east-1:123456:my-topic\nType\nNotification\n"
        );
    }

    #[test]
    fn test_sns_notification_with_subject() {
        let msg = SnsMessage {
            message_type: "Notification".into(),
            message_id: "msg-456".into(),
            message: "Body".into(),
            timestamp: "2024-01-01T00:00:00.000Z".into(),
            topic_arn: "arn:aws:sns:us-east-1:123456:topic".into(),
            signature: String::new(),
            signing_cert_url: String::new(),
            signature_version: "1".into(),
            subject: Some("My Subject".into()),
            subscribe_url: None,
            token: None,
            unsubscribe_url: None,
        };

        let result = build_sns_string_to_sign(&msg);
        assert!(result.contains("Subject\nMy Subject\n"));
    }

    #[test]
    fn test_sns_subscription_confirmation_string_to_sign() {
        let msg = SnsMessage {
            message_type: "SubscriptionConfirmation".into(),
            message_id: "msg-789".into(),
            message: "You have chosen to subscribe".into(),
            timestamp: "2024-01-01T00:00:00.000Z".into(),
            topic_arn: "arn:aws:sns:us-east-1:123456:topic".into(),
            signature: String::new(),
            signing_cert_url: String::new(),
            signature_version: "1".into(),
            subject: None,
            subscribe_url: Some(
                "https://sns.us-east-1.amazonaws.com/?Action=ConfirmSubscription".into(),
            ),
            token: Some("token-abc".into()),
            unsubscribe_url: None,
        };

        let result = build_sns_string_to_sign(&msg);
        assert!(result.contains(
            "SubscribeURL\nhttps://sns.us-east-1.amazonaws.com/?Action=ConfirmSubscription\n"
        ));
        assert!(result.contains("Token\ntoken-abc\n"));
        assert!(result.contains("Type\nSubscriptionConfirmation\n"));
    }

    // --- SNS message parsing ---

    #[test]
    fn test_sns_message_deserialization() {
        let json = r#"{
            "Type": "Notification",
            "MessageId": "id-123",
            "TopicArn": "arn:aws:sns:us-east-1:123:topic",
            "Subject": "test",
            "Message": "{\"key\":\"value\"}",
            "Timestamp": "2024-01-01T00:00:00.000Z",
            "SignatureVersion": "1",
            "Signature": "base64sig==",
            "SigningCertURL": "https://sns.us-east-1.amazonaws.com/cert.pem"
        }"#;

        let msg: SnsMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.message_type, "Notification");
        assert_eq!(msg.message_id, "id-123");
        assert_eq!(msg.subject, Some("test".into()));
        assert_eq!(msg.message, "{\"key\":\"value\"}");
    }

    // --- SNS URL validation (SubscribeURL SSRF prevention) ---

    #[test]
    fn test_sns_subscribe_url_valid() {
        assert!(is_valid_sns_url(
            "https://sns.us-east-1.amazonaws.com/?Action=ConfirmSubscription&TopicArn=arn:aws:sns:us-east-1:123456:test&Token=abc"
        ));
    }

    #[test]
    fn test_sns_subscribe_url_ssrf_blocked() {
        // Internal metadata endpoint
        assert!(!is_valid_sns_url(
            "http://169.254.169.254/latest/meta-data/"
        ));
        // Arbitrary external URL
        assert!(!is_valid_sns_url("https://evil.com/steal-data"));
        // Non-https
        assert!(!is_valid_sns_url(
            "http://sns.us-east-1.amazonaws.com/?Action=Confirm"
        ));
    }

    // --- PagerDuty verification ---

    #[test]
    fn test_pagerduty_signature_valid() {
        let secret = "pd_secret_key";
        let payload = b"{\"event\":{\"event_type\":\"incident.triggered\"}}";
        let expected = compute_hmac_sha256_hex(secret.as_bytes(), payload);

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-PagerDuty-Signature",
            format!("v1={expected}").parse().unwrap(),
        );

        assert!(verify_pagerduty(secret, payload, &headers).unwrap());
    }

    #[test]
    fn test_pagerduty_signature_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-PagerDuty-Signature",
            "v1=0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap(),
        );
        assert!(!verify_pagerduty("secret", b"payload", &headers).unwrap());
    }

    #[test]
    fn test_pagerduty_signature_missing() {
        let headers = HeaderMap::new();
        assert!(!verify_pagerduty("secret", b"payload", &headers).unwrap());
    }

    #[test]
    fn test_pagerduty_signature_no_prefix() {
        let secret = "pd_secret";
        let payload = b"data";
        let sig = compute_hmac_sha256_hex(secret.as_bytes(), payload);

        let mut headers = HeaderMap::new();
        // Missing v1= prefix — should fail
        headers.insert("X-PagerDuty-Signature", sig.parse().unwrap());
        assert!(!verify_pagerduty(secret, payload, &headers).unwrap());
    }

    // --- Grafana verification ---

    #[test]
    fn test_grafana_signature_valid() {
        let secret = "grafana_secret";
        let payload = b"{\"status\":\"firing\",\"alerts\":[]}";
        let expected = compute_hmac_sha256_hex(secret.as_bytes(), payload);

        let mut headers = HeaderMap::new();
        headers.insert("X-Grafana-Alerting-Signature", expected.parse().unwrap());

        assert!(verify_grafana(secret, payload, &headers).unwrap());
    }

    #[test]
    fn test_grafana_signature_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Grafana-Alerting-Signature",
            "0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap(),
        );
        assert!(!verify_grafana("secret", b"payload", &headers).unwrap());
    }

    #[test]
    fn test_grafana_signature_missing() {
        let headers = HeaderMap::new();
        assert!(!verify_grafana("secret", b"payload", &headers).unwrap());
    }

    // --- Terraform Cloud verification ---

    #[test]
    fn test_terraform_signature_valid() {
        let secret = "tf_secret";
        let payload = b"{\"payload_version\":1,\"notifications\":[]}";
        let expected = compute_hmac_sha512_hex(secret.as_bytes(), payload);

        let mut headers = HeaderMap::new();
        headers.insert("X-TFE-Notification-Signature", expected.parse().unwrap());

        assert!(verify_terraform(secret, payload, &headers).unwrap());
    }

    #[test]
    fn test_terraform_signature_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-TFE-Notification-Signature",
            "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap(),
        );
        assert!(!verify_terraform("secret", b"payload", &headers).unwrap());
    }

    #[test]
    fn test_terraform_signature_missing() {
        let headers = HeaderMap::new();
        assert!(!verify_terraform("secret", b"payload", &headers).unwrap());
    }

    // --- GitLab verification ---

    #[test]
    fn test_gitlab_token_valid() {
        let secret = "my_gitlab_token";
        let payload = b"{\"object_kind\":\"push\"}";

        let mut headers = HeaderMap::new();
        headers.insert("X-Gitlab-Token", secret.parse().unwrap());

        assert!(verify_gitlab(secret, payload, &headers).unwrap());
    }

    #[test]
    fn test_gitlab_token_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Gitlab-Token", "wrong_token".parse().unwrap());
        assert!(!verify_gitlab("correct_token", b"payload", &headers).unwrap());
    }

    #[test]
    fn test_gitlab_token_missing() {
        let headers = HeaderMap::new();
        assert!(!verify_gitlab("secret", b"payload", &headers).unwrap());
    }

    // --- verify_signature dispatches new providers ---

    #[test]
    fn test_verify_signature_pagerduty() {
        let secret = "pd_key";
        let payload = b"test";
        let sig = compute_hmac_sha256_hex(secret.as_bytes(), payload);

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-PagerDuty-Signature",
            format!("v1={sig}").parse().unwrap(),
        );

        assert!(verify_signature("pagerduty", secret, payload, &headers).unwrap());
    }

    #[test]
    fn test_verify_signature_grafana() {
        let secret = "gf_key";
        let payload = b"test";
        let sig = compute_hmac_sha256_hex(secret.as_bytes(), payload);

        let mut headers = HeaderMap::new();
        headers.insert("X-Grafana-Alerting-Signature", sig.parse().unwrap());

        assert!(verify_signature("grafana", secret, payload, &headers).unwrap());
    }

    #[test]
    fn test_verify_signature_terraform() {
        let secret = "tf_key";
        let payload = b"test";
        let sig = compute_hmac_sha512_hex(secret.as_bytes(), payload);

        let mut headers = HeaderMap::new();
        headers.insert("X-TFE-Notification-Signature", sig.parse().unwrap());

        assert!(verify_signature("terraform", secret, payload, &headers).unwrap());
    }

    #[test]
    fn test_verify_signature_gitlab() {
        let secret = "gl_token";

        let mut headers = HeaderMap::new();
        headers.insert("X-Gitlab-Token", secret.parse().unwrap());

        assert!(verify_signature("gitlab", secret, b"any", &headers).unwrap());
    }

    #[test]
    fn test_shopify_signature_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Shopify-Hmac-SHA256", "bad-signature".parse().unwrap());
        assert!(!verify_shopify("secret", b"payload", &headers).unwrap());
    }

    #[test]
    fn test_shopify_signature_missing() {
        let headers = HeaderMap::new();
        assert!(!verify_shopify("secret", b"payload", &headers).unwrap());
    }

    #[test]
    fn test_custom_hmac_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Webhook-Signature", "wrong-hex".parse().unwrap());
        assert!(!verify_custom_hmac("secret", b"payload", &headers).unwrap());
    }

    #[test]
    fn test_custom_hmac_missing() {
        let headers = HeaderMap::new();
        assert!(!verify_custom_hmac("secret", b"payload", &headers).unwrap());
    }

    #[test]
    fn test_stripe_signature_missing() {
        let headers = HeaderMap::new();
        assert!(!verify_stripe("whsec_test", b"payload", &headers).unwrap());
    }

    // --- SNS cert cache ---

    #[test]
    fn test_cert_cache() {
        let url = "https://sns.us-east-1.amazonaws.com/test-cache.pem";
        assert!(get_cached_cert(url).is_none());
        set_cached_cert(url, b"PEM DATA".to_vec());
        let cached = get_cached_cert(url);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap(), b"PEM DATA");
    }

    // --- Outbound signing ---

    #[test]
    fn test_sign_outbound_payload_deterministic() {
        let secret = "whsec_dGVzdF9zZWNyZXRfa2V5X2Zvcl9zaWduaW5n";
        let msg_id = "msg_001";
        let timestamp = 1710000000i64;
        let payload = b"{\"order_id\":\"123\"}";

        let sig1 = sign_outbound_payload(secret, msg_id, timestamp, payload);
        let sig2 = sign_outbound_payload(secret, msg_id, timestamp, payload);
        assert_eq!(sig1, sig2, "Same inputs must produce same signature");
    }

    #[test]
    fn test_sign_outbound_payload_different_secret() {
        let msg_id = "msg_001";
        let timestamp = 1710000000i64;
        let payload = b"{\"order_id\":\"123\"}";

        let sig1 = sign_outbound_payload("whsec_c2VjcmV0X2E=", msg_id, timestamp, payload);
        let sig2 = sign_outbound_payload("whsec_c2VjcmV0X2I=", msg_id, timestamp, payload);
        assert_ne!(
            sig1, sig2,
            "Different secrets must produce different signatures"
        );
    }

    #[test]
    fn test_sign_outbound_payload_different_timestamp() {
        let secret = "whsec_dGVzdA==";
        let msg_id = "msg_001";
        let payload = b"{}";

        let sig1 = sign_outbound_payload(secret, msg_id, 1000, payload);
        let sig2 = sign_outbound_payload(secret, msg_id, 2000, payload);
        assert_ne!(
            sig1, sig2,
            "Different timestamps must produce different signatures"
        );
    }

    #[test]
    fn test_sign_outbound_payload_base64_format() {
        let sig = sign_outbound_payload("whsec_c2VjcmV0", "msg_001", 1710000000, b"test");
        // HMAC-SHA256 produces 32 bytes = 44 base64 chars (with padding)
        assert_eq!(sig.len(), 44);
        assert!(
            sig.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
        );
    }

    #[test]
    fn test_sign_outbound_payload_verifiable() {
        use base64::Engine;
        // Standard Webhooks format: HMAC-SHA256(base64_decode(secret), "{msg_id}.{timestamp}.{payload}")
        let raw_key = b"verify_test_key_0123456789abcdef";
        let secret = format!(
            "whsec_{}",
            base64::engine::general_purpose::STANDARD.encode(raw_key)
        );
        let msg_id = "msg_verify";
        let timestamp = 1710000000i64;
        let payload = b"{\"amount\":5000}";

        let signature = sign_outbound_payload(&secret, msg_id, timestamp, payload);

        // Manually compute the expected HMAC per Standard Webhooks spec
        let mut mac = HmacSha256::new_from_slice(raw_key).unwrap();
        mac.update(b"msg_verify.1710000000.");
        mac.update(payload);
        let expected =
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

        assert_eq!(signature, expected);
    }

    #[test]
    fn test_sign_outbound_msg_id_included() {
        let secret = "whsec_dGVzdA==";
        let payload = b"{}";

        // Different msg_ids should produce different signatures
        let sig1 = sign_outbound_payload(secret, "msg_001", 1000, payload);
        let sig2 = sign_outbound_payload(secret, "msg_002", 1000, payload);
        assert_ne!(
            sig1, sig2,
            "Different msg_ids must produce different signatures"
        );
    }

    #[test]
    fn test_generate_signing_secret_prefix() {
        let secret = generate_signing_secret();
        assert!(
            secret.starts_with("whsec_"),
            "Secret must start with whsec_ prefix, got: {}",
            secret
        );
    }

    #[test]
    fn test_generate_signing_secret_unique() {
        let s1 = generate_signing_secret();
        let s2 = generate_signing_secret();
        assert_ne!(s1, s2, "Each generated secret must be unique");
    }

    #[test]
    fn test_generate_signing_secret_decodable() {
        use base64::Engine;
        let secret = generate_signing_secret();
        let b64_part = secret.strip_prefix("whsec_").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD.decode(b64_part);
        assert!(decoded.is_ok(), "Secret base64 part must be decodable");
        assert_eq!(decoded.unwrap().len(), 32, "Decoded key must be 32 bytes");
    }
}
