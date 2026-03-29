use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;

// ---------------------------------------------------------------------------
// 1. Config parsing
// ---------------------------------------------------------------------------
fn bench_config_parse(c: &mut Criterion) {
    let yaml = r#"
database:
  driver: sqlite
server:
  allow_private_urls: true
sources:
  github:
    type: webhook
    verify: github
    secret: test-secret
  stripe:
    type: webhook
    verify: stripe
    secret: test-secret
handlers:
  deploy:
    source: github
    events: [push]
    url: http://localhost:3000/deploy
    retry: { max: 5 }
  billing:
    source: stripe
    events: [invoice.paid]
    url: http://localhost:3000/billing
"#;
    c.bench_function("config_parse", |b| {
        b.iter(|| {
            let config: qhook::config::Config = serde_yaml_ng::from_str(black_box(yaml)).unwrap();
            config.validate().unwrap();
            black_box(config)
        })
    });
}

// ---------------------------------------------------------------------------
// 2. Filter evaluation
// ---------------------------------------------------------------------------
fn make_payload(field_count: usize) -> String {
    let mut fields: Vec<String> = Vec::with_capacity(field_count);
    for i in 0..field_count {
        fields.push(format!(r#""field_{}": "value_{}""#, i, i));
    }
    // Always include keys used in filters
    fields.push(r#""action": "opened""#.to_string());
    fields.push(r#""nested": {"deep": {"key": "found"}}"#.to_string());
    fields.push(r#""message": "hello world from qhook""#.to_string());
    fields.push(r#""count": 42"#.to_string());
    format!("{{{}}}", fields.join(","))
}

fn bench_filter_evaluation(c: &mut Criterion) {
    let payloads = vec![
        ("small_5", make_payload(5)),
        ("medium_50", make_payload(50)),
        ("large_500", make_payload(500)),
    ];

    let filters = vec![
        ("equality", "$.action == 'opened'"),
        ("nested_path", "$.nested.deep.key == 'found'"),
        ("contains", "$.message contains 'qhook'"),
        ("regex", "$.message matches 'hello.*qhook'"),
    ];

    let mut group = c.benchmark_group("filter_evaluation");
    for (payload_name, payload) in &payloads {
        for (filter_name, filter) in &filters {
            group.bench_with_input(
                BenchmarkId::new(*filter_name, *payload_name),
                &(payload.as_str(), *filter),
                |b, &(payload, filter)| {
                    b.iter(|| {
                        black_box(qhook::api::evaluate_filter_pub(
                            black_box(payload),
                            black_box(filter),
                        ))
                    })
                },
            );
        }
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// 3. ULID generation
// ---------------------------------------------------------------------------
fn bench_ulid_generation(c: &mut Criterion) {
    c.bench_function("ulid_generate", |b| {
        b.iter(|| black_box(ulid::Ulid::new().to_string()))
    });
}

// ---------------------------------------------------------------------------
// 4. Signature verification
// ---------------------------------------------------------------------------
fn bench_signature_verification(c: &mut Criterion) {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    let sizes: Vec<(&str, Vec<u8>)> = vec![
        ("256B", vec![b'x'; 256]),
        ("4KB", vec![b'x'; 4096]),
        ("64KB", vec![b'x'; 65536]),
    ];

    let github_secret = "test-github-secret";
    let stripe_secret = "test-stripe-secret";
    let sw_raw_key = b"standard_webhooks_key_bench_1234";
    let sw_secret = format!(
        "whsec_{}",
        base64::engine::general_purpose::STANDARD.encode(sw_raw_key)
    );

    let mut group = c.benchmark_group("signature_verification");

    for (size_name, payload) in &sizes {
        // --- GitHub ---
        let gh_sig = {
            let mut mac = HmacSha256::new_from_slice(github_secret.as_bytes()).unwrap();
            mac.update(payload);
            format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
        };
        let gh_headers = {
            let mut h = axum::http::HeaderMap::new();
            h.insert("X-Hub-Signature-256", gh_sig.parse().unwrap());
            h
        };

        group.bench_with_input(
            BenchmarkId::new("github", *size_name),
            &(payload.as_slice(), &gh_headers),
            |b, &(payload, headers)| {
                b.iter(|| {
                    black_box(
                        qhook::verify::verify_signature(
                            "github",
                            github_secret,
                            black_box(payload),
                            headers,
                        )
                        .unwrap(),
                    )
                })
            },
        );

        // --- Stripe ---
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let stripe_sig = {
            let mut mac = HmacSha256::new_from_slice(stripe_secret.as_bytes()).unwrap();
            mac.update(timestamp.as_bytes());
            mac.update(b".");
            mac.update(payload);
            format!(
                "t={},v1={}",
                timestamp,
                hex::encode(mac.finalize().into_bytes())
            )
        };
        let stripe_headers = {
            let mut h = axum::http::HeaderMap::new();
            h.insert("Stripe-Signature", stripe_sig.parse().unwrap());
            h
        };

        group.bench_with_input(
            BenchmarkId::new("stripe", *size_name),
            &(payload.as_slice(), &stripe_headers),
            |b, &(payload, headers)| {
                b.iter(|| {
                    black_box(
                        qhook::verify::verify_signature(
                            "stripe",
                            stripe_secret,
                            black_box(payload),
                            headers,
                        )
                        .unwrap(),
                    )
                })
            },
        );

        // --- Standard Webhooks ---
        let sw_timestamp = chrono::Utc::now().timestamp().to_string();
        let sw_msg_id = "msg_bench";
        let sw_sig = {
            let to_sign = format!(
                "{}.{}.{}",
                sw_msg_id,
                sw_timestamp,
                String::from_utf8_lossy(payload)
            );
            let mut mac = HmacSha256::new_from_slice(sw_raw_key).unwrap();
            mac.update(to_sign.as_bytes());
            let sig_b64 =
                base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
            format!("v1,{}", sig_b64)
        };
        let sw_headers = {
            let mut h = axum::http::HeaderMap::new();
            h.insert("webhook-id", sw_msg_id.parse().unwrap());
            h.insert("webhook-timestamp", sw_timestamp.parse().unwrap());
            h.insert("webhook-signature", sw_sig.parse().unwrap());
            h
        };

        group.bench_with_input(
            BenchmarkId::new("standard_webhooks", *size_name),
            &(payload.as_slice(), &sw_headers),
            |b, &(payload, headers)| {
                b.iter(|| {
                    black_box(
                        qhook::verify::verify_signature(
                            "standard-webhooks",
                            &sw_secret,
                            black_box(payload),
                            headers,
                        )
                        .unwrap(),
                    )
                })
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// 5. Event insertion (SQLite in-memory)
// ---------------------------------------------------------------------------
fn bench_event_insert(c: &mut Criterion) {
    use qhook::config::DatabaseConfig;
    use tokio::runtime::Runtime;

    let rt = Runtime::new().unwrap();

    let db = rt.block_on(async {
        let config = DatabaseConfig {
            driver: "sqlite".into(),
            url: Some("sqlite::memory:".into()),
            max_connections: 1,
            account_id: None,
            database_id: None,
            api_token: None,
            d1_endpoint: None,
        };
        let db = qhook::db::Database::connect(&config).await.unwrap();
        db.migrate().await.unwrap();
        db
    });

    let payload = serde_json::json!({
        "action": "push",
        "ref": "refs/heads/main",
        "repository": {"full_name": "user/repo"},
        "sender": {"login": "benchmark-user"},
        "commits": [{"id": "abc123", "message": "bench commit"}]
    })
    .to_string();

    let mut counter: u64 = 0;

    c.bench_function("event_insert_sqlite", |b| {
        b.iter(|| {
            counter += 1;
            let id = ulid::Ulid::new().to_string();
            rt.block_on(async {
                black_box(
                    db.insert_event(&id, "bench-source", "push", &payload, None, None)
                        .await
                        .unwrap(),
                )
            })
        })
    });

    let _ = counter; // suppress unused warning
}

// ---------------------------------------------------------------------------
// Criterion groups
// ---------------------------------------------------------------------------
criterion_group!(
    benches,
    bench_config_parse,
    bench_filter_evaluation,
    bench_ulid_generation,
    bench_signature_verification,
    bench_event_insert,
);
criterion_main!(benches);
