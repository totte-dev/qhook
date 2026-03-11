//! Shared test infrastructure for integration tests.
#![allow(dead_code)]

use std::process::Stdio;
use tokio::process::Command;
use wiremock::MockServer;

/// A running qhook process started from the binary with a temp config + DB.
pub struct QhookProcess {
    child: tokio::process::Child,
    pub base_url: String,
    config_path: String,
    pub db_path: String,
}

impl QhookProcess {
    /// Start qhook with the given YAML config on the specified port.
    ///
    /// The YAML must contain `__DB_PATH__` and `__PORT__` placeholders.
    pub async fn start(yaml: &str, port: u16) -> Self {
        let id = ulid::Ulid::new().to_string();
        let db_path = format!("/tmp/qhook_test_{}.db", id);
        let config_path = format!("/tmp/qhook_test_{}.yaml", id);

        let yaml = yaml
            .replace("__DB_PATH__", &db_path)
            .replace("__PORT__", &port.to_string());

        std::fs::write(&config_path, &yaml).unwrap();

        let child = Command::new(env!("CARGO_BIN_EXE_qhook"))
            .args(["start", "--config", &config_path])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("failed to start qhook");

        let base_url = format!("http://127.0.0.1:{}", port);

        let client = reqwest::Client::new();
        let mut ready = false;
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if let Ok(resp) = client.get(format!("{}/health", base_url)).send().await {
                if resp.status().is_success() {
                    ready = true;
                    break;
                }
            }
        }
        assert!(
            ready,
            "qhook failed to start on port {port} within 5 seconds"
        );

        Self {
            child,
            base_url,
            config_path,
            db_path,
        }
    }

    pub fn url(&self, p: &str) -> String {
        format!("{}{}", self.base_url, p)
    }

    pub async fn stop(mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
        let _ = std::fs::remove_file(&self.config_path);
        let _ = std::fs::remove_file(&self.db_path);
        let _ = std::fs::remove_file(format!("{}-wal", &self.db_path));
        let _ = std::fs::remove_file(format!("{}-shm", &self.db_path));
    }
}

pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap()
}

/// Wait until `mock` has received at least `expected` requests, or panic on timeout.
pub async fn wait_for_mock(mock: &MockServer, expected: usize, timeout_secs: u64) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        let n = mock.received_requests().await.unwrap_or_default().len();
        if n >= expected {
            return;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("Timed out: expected {} requests, got {}", expected, n);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

/// Count received requests matching a path substring.
pub async fn count_path(mock: &MockServer, path_contains: &str) -> usize {
    mock.received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| r.url.path().contains(path_contains))
        .count()
}

/// HMAC-SHA256 hex digest.
pub fn hmac_sha256(key: &str, data: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).unwrap();
    mac.update(data.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Verify a Standard Webhooks signature.
/// Returns true if the signature matches the expected HMAC-SHA256.
pub fn verify_standard_webhook_sig(
    signing_secret: &str,
    msg_id: &str,
    timestamp: &str,
    body: &[u8],
    signature_header: &str,
) -> bool {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let sig_b64 = match signature_header.strip_prefix("v1,") {
        Some(s) => s,
        None => return false,
    };
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(
            signing_secret
                .strip_prefix("whsec_")
                .unwrap_or(signing_secret),
        )
        .unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(&key_bytes).unwrap();
    mac.update(msg_id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    let expected = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    sig_b64 == expected
}
