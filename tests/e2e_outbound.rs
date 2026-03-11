//! E2E tests for outbound webhooks — endpoint management, signed delivery, and subscriptions.

mod common;

use common::{QhookProcess, http, wait_for_mock};
use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const BASE_YAML: &str = r#"
database:
  driver: sqlite
  url: "sqlite:__DB_PATH__?mode=rwc"
server:
  port: __PORT__
  allow_private_urls: true
api:
  auth_token: "test-token"
sources:
  my-saas:
    type: outbound
"#;

// --- Endpoint CRUD ---

#[tokio::test]
async fn outbound_create_endpoint_requires_outbound_source() {
    // Config with an event source, not outbound
    let yaml = r#"
database:
  driver: sqlite
  url: "sqlite:__DB_PATH__?mode=rwc"
server:
  port: __PORT__
  allow_private_urls: true
api:
  auth_token: "test-token"
sources:
  app:
    type: event
  my-saas:
    type: outbound
handlers:
  noop:
    source: app
    events: ["*"]
    url: http://localhost:9999/noop
"#;

    let server = QhookProcess::start(yaml, 19802).await;
    let client = http();

    // Try creating endpoint for non-outbound source
    let resp = client
        .post(server.url("/api/outbound/endpoints"))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "source": "app",
            "url": "https://customer.example.com/webhook",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("not an outbound source")
    );

    // Non-existent source
    let resp = client
        .post(server.url("/api/outbound/endpoints"))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "source": "nonexistent",
            "url": "https://customer.example.com/webhook",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    server.stop().await;
}

#[tokio::test]
async fn outbound_endpoint_crud_lifecycle() {
    let server = QhookProcess::start(BASE_YAML, 19803).await;
    let client = http();

    // Create
    let resp = client
        .post(server.url("/api/outbound/endpoints"))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "source": "my-saas",
            "url": "https://customer.example.com/webhook",
            "description": "Test endpoint"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: Value = resp.json().await.unwrap();
    let endpoint_id = body["id"].as_str().unwrap().to_string();

    // Get
    let resp = client
        .get(server.url(&format!("/api/outbound/endpoints/{}", endpoint_id)))
        .header("Authorization", "Bearer test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["id"], endpoint_id);
    assert_eq!(body["description"], "Test endpoint");
    // GET returns signing_secret
    assert!(
        body["signing_secret"]
            .as_str()
            .unwrap()
            .starts_with("whsec_")
    );

    // Update
    let resp = client
        .put(server.url(&format!("/api/outbound/endpoints/{}", endpoint_id)))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "url": "https://new-url.example.com/hook",
            "status": "disabled"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["url"], "https://new-url.example.com/hook");
    assert_eq!(body["status"], "disabled");

    // List
    let resp = client
        .get(server.url("/api/outbound/endpoints?source=my-saas"))
        .header("Authorization", "Bearer test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let endpoints = body["endpoints"].as_array().unwrap();
    assert_eq!(endpoints.len(), 1);
    // List does NOT return signing_secret
    assert!(endpoints[0].get("signing_secret").is_none());

    // Delete
    let resp = client
        .delete(server.url(&format!("/api/outbound/endpoints/{}", endpoint_id)))
        .header("Authorization", "Bearer test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    // Verify deleted
    let resp = client
        .get(server.url(&format!("/api/outbound/endpoints/{}", endpoint_id)))
        .header("Authorization", "Bearer test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    server.stop().await;
}

#[tokio::test]
async fn outbound_requires_auth() {
    let server = QhookProcess::start(BASE_YAML, 19804).await;
    let client = http();

    // No auth
    let resp = client
        .post(server.url("/api/outbound/endpoints"))
        .json(&serde_json::json!({
            "source": "my-saas",
            "url": "https://example.com/hook",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Wrong token
    let resp = client
        .post(server.url("/api/outbound/endpoints"))
        .header("Authorization", "Bearer wrong-token")
        .json(&serde_json::json!({
            "source": "my-saas",
            "url": "https://example.com/hook",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    server.stop().await;
}

// --- Subscriptions ---

#[tokio::test]
async fn outbound_subscription_management() {
    let server = QhookProcess::start(BASE_YAML, 19805).await;
    let client = http();

    // Create endpoint
    let resp = client
        .post(server.url("/api/outbound/endpoints"))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "source": "my-saas",
            "url": "https://customer.example.com/webhook",
        }))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let endpoint_id = body["id"].as_str().unwrap().to_string();

    // Add subscriptions
    let resp = client
        .post(server.url(&format!(
            "/api/outbound/endpoints/{}/subscriptions",
            endpoint_id
        )))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "event_types": ["order.created", "payment.completed"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: Value = resp.json().await.unwrap();
    let subs = body["subscriptions"].as_array().unwrap();
    assert_eq!(subs.len(), 2);
    let sub_id = subs[0]["id"].as_str().unwrap().to_string();

    // List subscriptions
    let resp = client
        .get(server.url(&format!(
            "/api/outbound/endpoints/{}/subscriptions",
            endpoint_id
        )))
        .header("Authorization", "Bearer test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["subscriptions"].as_array().unwrap().len(), 2);

    // Duplicate subscription is idempotent (no error, but no new row)
    let resp = client
        .post(server.url(&format!(
            "/api/outbound/endpoints/{}/subscriptions",
            endpoint_id
        )))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "event_types": ["order.created"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: Value = resp.json().await.unwrap();
    // Duplicate was skipped — 0 new subscriptions
    assert_eq!(body["subscriptions"].as_array().unwrap().len(), 0);

    // Delete subscription
    let resp = client
        .delete(server.url(&format!(
            "/api/outbound/endpoints/{}/subscriptions/{}",
            endpoint_id, sub_id
        )))
        .header("Authorization", "Bearer test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    // Verify only 1 subscription remains
    let resp = client
        .get(server.url(&format!(
            "/api/outbound/endpoints/{}/subscriptions",
            endpoint_id
        )))
        .header("Authorization", "Bearer test-token")
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["subscriptions"].as_array().unwrap().len(), 1);

    server.stop().await;
}

// --- Secret rotation ---

#[tokio::test]
async fn outbound_rotate_secret() {
    let server = QhookProcess::start(BASE_YAML, 19806).await;
    let client = http();

    // Create endpoint
    let resp = client
        .post(server.url("/api/outbound/endpoints"))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "source": "my-saas",
            "url": "https://customer.example.com/webhook",
        }))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let endpoint_id = body["id"].as_str().unwrap().to_string();
    let old_secret = body["signing_secret"].as_str().unwrap().to_string();

    // Rotate
    let resp = client
        .post(server.url(&format!(
            "/api/outbound/endpoints/{}/rotate-secret",
            endpoint_id
        )))
        .header("Authorization", "Bearer test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let new_secret = body["signing_secret"].as_str().unwrap().to_string();

    assert_ne!(
        old_secret, new_secret,
        "Secret should change after rotation"
    );
    assert!(new_secret.starts_with("whsec_"));

    // Verify GET returns new secret
    let resp = client
        .get(server.url(&format!("/api/outbound/endpoints/{}", endpoint_id)))
        .header("Authorization", "Bearer test-token")
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["signing_secret"], new_secret);

    server.stop().await;
}

// --- Outbound delivery with signature ---

#[tokio::test]
async fn outbound_signed_delivery() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;

    let server = QhookProcess::start(BASE_YAML, 19807).await;
    let client = http();

    // Create endpoint pointing to mock server
    let resp = client
        .post(server.url("/api/outbound/endpoints"))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "source": "my-saas",
            "url": format!("{}/webhook", mock.uri()),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: Value = resp.json().await.unwrap();
    let signing_secret = body["signing_secret"].as_str().unwrap().to_string();
    let endpoint_id = body["id"].as_str().unwrap().to_string();

    // Subscribe to order.created
    let resp = client
        .post(server.url(&format!(
            "/api/outbound/endpoints/{}/subscriptions",
            endpoint_id
        )))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "event_types": ["order.created"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // Send event
    let resp = client
        .post(server.url("/events/my-saas/order.created"))
        .header("Content-Type", "application/json")
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({"order_id": "ord_123", "amount": 5000}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);
    let event_body: Value = resp.json().await.unwrap();
    assert_eq!(event_body["jobs_created"], 1);

    // Wait for delivery
    wait_for_mock(&mock, 1, 10).await;

    let reqs = mock.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);

    // Verify payload
    let delivered: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(delivered["order_id"], "ord_123");
    assert_eq!(delivered["amount"], 5000);

    // Verify Standard Webhooks headers
    let sig_header = reqs[0]
        .headers
        .get("webhook-signature")
        .expect("webhook-signature header must be present")
        .to_str()
        .unwrap();
    assert!(
        sig_header.starts_with("v1,"),
        "Signature must have v1, prefix, got: {}",
        sig_header
    );

    let timestamp_header = reqs[0]
        .headers
        .get("webhook-timestamp")
        .expect("webhook-timestamp header must be present")
        .to_str()
        .unwrap();
    let timestamp: i64 = timestamp_header.parse().expect("Timestamp must be numeric");
    assert!(timestamp > 0);

    let msg_id = reqs[0]
        .headers
        .get("webhook-id")
        .expect("webhook-id header must be present")
        .to_str()
        .unwrap();

    // Verify the signature is valid HMAC-SHA256 per Standard Webhooks spec
    let sig_b64 = sig_header.strip_prefix("v1,").unwrap();
    let payload_bytes = reqs[0].body.as_ref();
    let expected_sig = {
        use base64::Engine;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        // Decode the whsec_ secret to raw key bytes
        let key_bytes = base64::engine::general_purpose::STANDARD
            .decode(signing_secret.strip_prefix("whsec_").unwrap())
            .unwrap();
        let mut mac = Hmac::<Sha256>::new_from_slice(&key_bytes).unwrap();
        // Standard Webhooks signed content: {msg_id}.{timestamp}.{body}
        mac.update(msg_id.as_bytes());
        mac.update(b".");
        mac.update(timestamp_header.as_bytes());
        mac.update(b".");
        mac.update(payload_bytes);
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    };
    assert_eq!(
        sig_b64,
        expected_sig,
        "Signature must match Standard Webhooks HMAC-SHA256 of '{}.{}.{}'",
        msg_id,
        timestamp_header,
        std::str::from_utf8(payload_bytes).unwrap_or("<binary>")
    );

    // Verify supplementary headers
    assert!(reqs[0].headers.get("X-Qhook-Event-ID").is_some());
    assert_eq!(
        reqs[0]
            .headers
            .get("X-Qhook-Event-Type")
            .unwrap()
            .to_str()
            .unwrap(),
        "order.created"
    );

    server.stop().await;
}

// --- Disabled endpoint skips delivery ---

#[tokio::test]
async fn outbound_disabled_endpoint_no_delivery() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0) // Should NOT receive any requests
        .mount(&mock)
        .await;

    let server = QhookProcess::start(BASE_YAML, 19808).await;
    let client = http();

    // Create endpoint
    let resp = client
        .post(server.url("/api/outbound/endpoints"))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "source": "my-saas",
            "url": format!("{}/webhook", mock.uri()),
        }))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let endpoint_id = body["id"].as_str().unwrap().to_string();

    // Subscribe to events
    client
        .post(server.url(&format!(
            "/api/outbound/endpoints/{}/subscriptions",
            endpoint_id
        )))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "event_types": ["order.created"]
        }))
        .send()
        .await
        .unwrap();

    // Disable endpoint
    client
        .put(server.url(&format!("/api/outbound/endpoints/{}", endpoint_id)))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({"status": "disabled"}))
        .send()
        .await
        .unwrap();

    // Send event — should NOT create any jobs
    let resp = client
        .post(server.url("/events/my-saas/order.created"))
        .header("Content-Type", "application/json")
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({"order_id": "ord_456"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["jobs_created"], 0,
        "Disabled endpoint should not create jobs"
    );

    // Wait briefly to ensure no delivery
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    server.stop().await;
}

// --- Wildcard subscription ---

#[tokio::test]
async fn outbound_wildcard_subscription() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;

    let server = QhookProcess::start(BASE_YAML, 19809).await;
    let client = http();

    // Create endpoint with wildcard subscription
    let resp = client
        .post(server.url("/api/outbound/endpoints"))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "source": "my-saas",
            "url": format!("{}/webhook", mock.uri()),
        }))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let endpoint_id = body["id"].as_str().unwrap().to_string();

    client
        .post(server.url(&format!(
            "/api/outbound/endpoints/{}/subscriptions",
            endpoint_id
        )))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "event_types": ["*"]
        }))
        .send()
        .await
        .unwrap();

    // Send two different event types
    client
        .post(server.url("/events/my-saas/order.created"))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({"type": "order"}))
        .send()
        .await
        .unwrap();

    client
        .post(server.url("/events/my-saas/payment.completed"))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({"type": "payment"}))
        .send()
        .await
        .unwrap();

    // Both should be delivered
    wait_for_mock(&mock, 2, 10).await;

    let reqs = mock.received_requests().await.unwrap();
    assert_eq!(
        reqs.len(),
        2,
        "Wildcard subscription should receive all event types"
    );

    server.stop().await;
}

// --- Multiple endpoints fan-out ---

#[tokio::test]
async fn outbound_multiple_endpoints_fanout() {
    let mock_a = MockServer::start().await;
    let mock_b = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/hook-a"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_a)
        .await;
    Mock::given(method("POST"))
        .and(path("/hook-b"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_b)
        .await;

    let server = QhookProcess::start(BASE_YAML, 19810).await;
    let client = http();

    // Create two endpoints
    let resp_a = client
        .post(server.url("/api/outbound/endpoints"))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "source": "my-saas",
            "url": format!("{}/hook-a", mock_a.uri()),
        }))
        .send()
        .await
        .unwrap();
    let body_a: Value = resp_a.json().await.unwrap();
    let ep_a = body_a["id"].as_str().unwrap().to_string();

    let resp_b = client
        .post(server.url("/api/outbound/endpoints"))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({
            "source": "my-saas",
            "url": format!("{}/hook-b", mock_b.uri()),
        }))
        .send()
        .await
        .unwrap();
    let body_b: Value = resp_b.json().await.unwrap();
    let ep_b = body_b["id"].as_str().unwrap().to_string();

    // Both subscribe to same event type
    for ep_id in [&ep_a, &ep_b] {
        client
            .post(server.url(&format!("/api/outbound/endpoints/{}/subscriptions", ep_id)))
            .header("Authorization", "Bearer test-token")
            .json(&serde_json::json!({
                "event_types": ["order.created"]
            }))
            .send()
            .await
            .unwrap();
    }

    // Send one event
    let resp = client
        .post(server.url("/events/my-saas/order.created"))
        .header("Authorization", "Bearer test-token")
        .json(&serde_json::json!({"order_id": "ord_789"}))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["jobs_created"], 2,
        "Should create job for each subscribed endpoint"
    );

    // Both endpoints should receive the event
    wait_for_mock(&mock_a, 1, 10).await;
    wait_for_mock(&mock_b, 1, 10).await;

    let reqs_a = mock_a.received_requests().await.unwrap();
    let reqs_b = mock_b.received_requests().await.unwrap();
    assert_eq!(reqs_a.len(), 1);
    assert_eq!(reqs_b.len(), 1);

    // Both receive the same payload
    let body_a: Value = serde_json::from_slice(&reqs_a[0].body).unwrap();
    let body_b: Value = serde_json::from_slice(&reqs_b[0].body).unwrap();
    assert_eq!(body_a["order_id"], "ord_789");
    assert_eq!(body_b["order_id"], "ord_789");

    server.stop().await;
}
