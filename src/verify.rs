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

pub async fn verify_sns_message(
    msg: &SnsMessage,
    http: &reqwest::Client,
) -> Result<bool> {
    let cert_url = &msg.signing_cert_url;

    // Validate the signing cert URL is from SNS
    if !is_valid_sns_cert_url(cert_url) {
        tracing::warn!(url = cert_url, "Invalid SNS SigningCertURL");
        return Ok(false);
    }

    // Fetch the signing certificate
    let pem_data = http
        .get(cert_url)
        .send()
        .await?
        .bytes()
        .await?;

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
            tracing::warn!(version = msg.signature_version, "Unknown SNS SignatureVersion");
            false
        }
    };

    Ok(valid)
}

pub fn is_valid_sns_cert_url(url: &str) -> bool {
    // Must be HTTPS from an SNS endpoint
    if !url.starts_with("https://sns.") {
        return false;
    }
    // Must be from amazonaws.com
    if let Some(host_start) = url.strip_prefix("https://") {
        if let Some(path_start) = host_start.find('/') {
            let host = &host_start[..path_start];
            return host.ends_with(".amazonaws.com");
        }
    }
    false
}

pub fn build_sns_string_to_sign(msg: &SnsMessage) -> String {
    let mut lines = String::new();

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
        let timestamp = "1234567890";
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
        assert!(is_valid_sns_cert_url(
            "https://sns.us-east-1.amazonaws.com/SimpleNotificationService-abc123.pem"
        ));
        assert!(is_valid_sns_cert_url(
            "https://sns.ap-northeast-1.amazonaws.com/cert.pem"
        ));
    }

    #[test]
    fn test_sns_cert_url_invalid() {
        assert!(!is_valid_sns_cert_url("http://sns.us-east-1.amazonaws.com/cert.pem")); // http
        assert!(!is_valid_sns_cert_url("https://evil.com/cert.pem")); // wrong domain
        assert!(!is_valid_sns_cert_url("https://sns.us-east-1.evil.com/cert.pem")); // spoofed
        assert!(!is_valid_sns_cert_url("https://amazonaws.com/cert.pem")); // missing sns prefix
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
            subscribe_url: Some("https://sns.us-east-1.amazonaws.com/?Action=ConfirmSubscription".into()),
            token: Some("token-abc".into()),
            unsubscribe_url: None,
        };

        let result = build_sns_string_to_sign(&msg);
        assert!(result.contains("SubscribeURL\nhttps://sns.us-east-1.amazonaws.com/?Action=ConfirmSubscription\n"));
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
}
