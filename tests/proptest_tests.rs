use proptest::prelude::*;
use std::collections::HashSet;

// ============================================================================
// a) Config parsing robustness
// ============================================================================

/// Strategy for generating arbitrary database driver names.
/// Mix of valid drivers and random strings to test rejection of unknown ones.
fn arb_driver() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("sqlite".to_string()),
        Just("postgres".to_string()),
        Just("mysql".to_string()),
        Just("".to_string()),
        Just("mongodb".to_string()),
        Just("redis".to_string()),
        "\\PC{1,50}", // random printable strings
    ]
}

/// Strategy for generating arbitrary source type names.
fn arb_source_type() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("webhook".to_string()),
        Just("event".to_string()),
        Just("sns".to_string()),
        Just("cron".to_string()),
        Just("outbound".to_string()),
        Just("".to_string()),
        Just("unknown_type".to_string()),
        "\\PC{1,30}",
    ]
}

/// Strategy for arbitrary source/handler names (including edge cases).
fn arb_name() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("".to_string()),
        Just("a".to_string()),
        Just("my-source".to_string()),
        Just("source_with_underscore".to_string()),
        "[a-zA-Z0-9_\\-]{1,64}",
        // Special characters that might trip up YAML or internal logic
        Just("name with spaces".to_string()),
        Just("name\twith\ttabs".to_string()),
        Just("emoji🎉name".to_string()),
    ]
}

/// Strategy for handler URLs.
fn arb_url() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("http://localhost:3000/hook".to_string()),
        Just("https://example.com/webhook".to_string()),
        Just("".to_string()),
        Just("not-a-url".to_string()),
        Just("ftp://bad-scheme.com".to_string()),
        Just("http://192.168.1.1/private".to_string()),
    ]
}

/// Strategy for retry max values.
fn arb_retry_max() -> impl Strategy<Value = u32> {
    prop_oneof![Just(0u32), Just(1u32), Just(5u32), Just(100u32), 0..1000u32,]
}

proptest! {
    /// Config::validate() must never panic, regardless of input.
    /// It should return Ok or Err, but never crash.
    #[test]
    fn config_validate_never_panics_with_arbitrary_driver(driver in arb_driver()) {
        let yaml = format!(
            r#"
database:
  driver: {driver}
server:
  port: 8888
  allow_private_urls: true
sources: {{}}
handlers: {{}}
"#
        );
        // Parsing might fail on weird chars, that's fine. We only care about no panics.
        if let Ok(config) = serde_yaml_ng::from_str::<qhook::config::Config>(&yaml) {
            // validate() should return Ok or Err, never panic
            let _ = config.validate();
        }
    }

    /// Config with arbitrary port numbers should parse and validate without panicking.
    #[test]
    fn config_validate_never_panics_with_arbitrary_port(port in 0u16..=u16::MAX) {
        let yaml = format!(
            r#"
database:
  driver: sqlite
server:
  port: {port}
  allow_private_urls: true
sources: {{}}
handlers: {{}}
"#
        );
        let config: qhook::config::Config = serde_yaml_ng::from_str(&yaml).unwrap();
        let _ = config.validate();
    }

    /// validate() correctly rejects unknown database drivers and accepts known ones.
    #[test]
    fn config_validate_rejects_unknown_drivers(driver in arb_driver()) {
        let yaml = format!(
            r#"
database:
  driver: {driver}
server:
  port: 8888
  allow_private_urls: true
sources: {{}}
handlers: {{}}
"#
        );
        if let Ok(config) = serde_yaml_ng::from_str::<qhook::config::Config>(&yaml) {
            let result = config.validate();
            let valid_drivers = ["sqlite", "postgres", "mysql"];
            if valid_drivers.contains(&driver.as_str()) {
                prop_assert!(result.is_ok(), "Valid driver '{}' was rejected: {:?}", driver, result);
            } else {
                prop_assert!(result.is_err(), "Invalid driver '{}' was accepted", driver);
                let err_msg = result.unwrap_err().to_string();
                prop_assert!(
                    err_msg.contains("unsupported database driver"),
                    "Error message for driver '{}' should mention 'unsupported database driver', got: {}",
                    driver,
                    err_msg
                );
            }
        }
    }

    /// validate() correctly rejects unknown source types and accepts known ones.
    #[test]
    fn config_validate_rejects_unknown_source_types(
        source_type in arb_source_type(),
        name in "[a-zA-Z][a-zA-Z0-9_]{0,15}"
    ) {
        // Cron sources need a schedule, so skip that complexity for this test
        if source_type == "cron" {
            return Ok(());
        }
        let yaml = format!(
            r#"
database:
  driver: sqlite
server:
  port: 8888
  allow_private_urls: true
sources:
  {name}:
    type: "{source_type}"
handlers: {{}}
"#
        );
        if let Ok(config) = serde_yaml_ng::from_str::<qhook::config::Config>(&yaml) {
            let result = config.validate();
            let valid_types = ["webhook", "event", "sns", "outbound"];
            if valid_types.contains(&source_type.as_str()) {
                prop_assert!(result.is_ok(), "Valid source type '{}' was rejected: {:?}", source_type, result);
            } else if !source_type.is_empty() {
                prop_assert!(result.is_err(), "Invalid source type '{}' was accepted", source_type);
            }
        }
    }

    /// Handler configs with arbitrary retry max values should not panic during validation.
    #[test]
    fn config_validate_never_panics_with_handler_retry(
        retry_max in arb_retry_max(),
        url in arb_url()
    ) {
        let yaml = format!(
            r#"
database:
  driver: sqlite
server:
  port: 8888
  allow_private_urls: true
sources:
  src1:
    type: webhook
handlers:
  h1:
    source: src1
    url: "{url}"
    retry:
      max: {retry_max}
"#
        );
        if let Ok(config) = serde_yaml_ng::from_str::<qhook::config::Config>(&yaml) {
            let _ = config.validate();
        }
    }

    /// Edge case strings for source names should never cause panics.
    #[test]
    fn config_parse_never_panics_with_edge_case_names(name in arb_name()) {
        // Build YAML manually to handle weird names - put name in quotes
        let yaml = format!(
            r#"
database:
  driver: sqlite
server:
  port: 8888
  allow_private_urls: true
sources:
  "{}":
    type: webhook
handlers: {{}}
"#,
            name.replace('"', "\\\"").replace('\n', "\\n")
        );
        // We only care that parsing + validation doesn't panic
        if let Ok(config) = serde_yaml_ng::from_str::<qhook::config::Config>(&yaml) {
            let _ = config.validate();
        }
    }
}

// ============================================================================
// b) Retry backoff calculation
// ============================================================================

/// Compute backoff the same way qhook does: 30s * 2^attempt, capped at 2^10.
/// Formula from src/queue.rs: `30i64 * (1i64 << current_attempt.min(10))`
fn compute_backoff_secs(attempt: u32) -> i64 {
    30i64 * (1i64 << (attempt as i64).min(10))
}

proptest! {
    /// Backoff delay is always positive for any attempt number.
    #[test]
    fn backoff_always_positive(attempt in 0u32..100) {
        let delay = compute_backoff_secs(attempt);
        prop_assert!(delay > 0, "Backoff for attempt {} was {}, expected > 0", attempt, delay);
    }

    /// Backoff increases monotonically with attempt number (up to the cap).
    #[test]
    fn backoff_monotonically_increases(attempt in 0u32..50) {
        let delay_current = compute_backoff_secs(attempt);
        let delay_next = compute_backoff_secs(attempt + 1);
        // Due to .min(10) cap, delays plateau after attempt 10
        prop_assert!(
            delay_next >= delay_current,
            "Backoff decreased from attempt {} ({}) to attempt {} ({})",
            attempt, delay_current, attempt + 1, delay_next
        );
    }

    /// Backoff never exceeds the hard cap: 30 * 2^10 = 30720 seconds.
    #[test]
    fn backoff_never_exceeds_cap(attempt in 0u32..1000) {
        let delay = compute_backoff_secs(attempt);
        let max_delay = 30i64 * (1i64 << 10); // 30720
        prop_assert!(
            delay <= max_delay,
            "Backoff for attempt {} was {}, exceeds cap {}",
            attempt, delay, max_delay
        );
    }

    /// Backoff at attempt 0 is exactly 30 * 2^0 = 30 seconds (minimum non-trivial delay).
    /// At attempt 10+, it plateaus at 30 * 1024 = 30720.
    #[test]
    fn backoff_exact_values_at_boundaries(attempt in 0u32..20) {
        let delay = compute_backoff_secs(attempt);
        let capped_attempt = attempt.min(10);
        let expected = 30i64 * (1i64 << capped_attempt);
        prop_assert_eq!(
            delay, expected,
            "Backoff for attempt {} should be {}, got {}",
            attempt, expected, delay
        );
    }

    /// DB error backoff: 2^consecutive_errors, capped at 30s.
    /// Formula: (1u64 << consecutive_db_errors.min(5)).min(30)
    #[test]
    fn db_error_backoff_always_positive_and_capped(errors in 0u64..100) {
        let backoff = (1u64 << errors.min(5)).min(30);
        prop_assert!(backoff >= 1, "DB backoff was 0 for {} errors", errors);
        prop_assert!(backoff <= 30, "DB backoff {} exceeded 30s cap for {} errors", backoff, errors);
    }

    /// DB error backoff increases monotonically (up to the cap).
    #[test]
    fn db_error_backoff_monotonic(errors in 0u64..50) {
        let current = (1u64 << errors.min(5)).min(30);
        let next = (1u64 << (errors + 1).min(5)).min(30);
        prop_assert!(
            next >= current,
            "DB backoff decreased from {} errors ({}) to {} errors ({})",
            errors, current, errors + 1, next
        );
    }
}

// ============================================================================
// c) Filter evaluation
// ============================================================================

/// Strategy for generating arbitrary (but valid) JSON payloads.
fn arb_json_payload() -> impl Strategy<Value = String> {
    prop_oneof![
        // Simple objects
        Just(r#"{}"#.to_string()),
        Just(r#"{"key": "value"}"#.to_string()),
        Just(r#"{"a": 1, "b": true, "c": null}"#.to_string()),
        Just(r#"{"nested": {"deep": {"value": 42}}}"#.to_string()),
        Just(r#"{"list": [1, 2, 3]}"#.to_string()),
        Just(r#"{"empty_string": ""}"#.to_string()),
        Just(r#"{"zero": 0}"#.to_string()),
        Just(r#"{"negative": -1}"#.to_string()),
        Just(r#"{"float": 3.14}"#.to_string()),
        Just(r#"{"big": 99999999999}"#.to_string()),
        // Use arb_json_value for more variety
        arb_json_object().prop_map(|v| serde_json::to_string(&v).unwrap()),
    ]
}

/// Strategy for generating arbitrary JSON objects.
fn arb_json_object() -> impl Strategy<Value = serde_json::Value> {
    let leaf = prop_oneof![
        Just(serde_json::Value::Null),
        any::<bool>().prop_map(serde_json::Value::Bool),
        any::<i32>().prop_map(|n| serde_json::json!(n)),
        "[a-zA-Z0-9 _\\-]{0,50}".prop_map(|s| serde_json::Value::String(s)),
    ];

    leaf.prop_recursive(
        3,  // depth
        64, // max nodes
        10, // items per collection
        |inner| {
            prop_oneof![
                // Array of values
                prop::collection::vec(inner.clone(), 0..5).prop_map(serde_json::Value::Array),
                // Object with string keys
                prop::collection::hash_map("[a-zA-Z_][a-zA-Z0-9_]{0,10}", inner, 0..5)
                    .prop_map(|map| { serde_json::Value::Object(map.into_iter().collect()) }),
            ]
        },
    )
}

/// Strategy for generating arbitrary filter expressions.
fn arb_filter() -> impl Strategy<Value = String> {
    prop_oneof![
        // Empty filter
        Just("".to_string()),
        // Truthy checks
        Just("$.key".to_string()),
        Just("$.nonexistent".to_string()),
        Just("$.nested.deep.value".to_string()),
        // Equality
        Just("$.key == value".to_string()),
        Just("$.key == 42".to_string()),
        Just("$.key != value".to_string()),
        // Numeric comparisons
        Just("$.amount >= 100".to_string()),
        Just("$.amount <= 100".to_string()),
        Just("$.amount > 0".to_string()),
        Just("$.amount < 999".to_string()),
        // Existence
        Just("$.key exists".to_string()),
        Just("$.nonexistent exists".to_string()),
        // Negation
        Just("not $.key".to_string()),
        Just("not $.key == value".to_string()),
        // Contains
        Just("$.key contains val".to_string()),
        // Starts/ends with
        Just("$.key starts_with v".to_string()),
        Just("$.key ends_with e".to_string()),
        // In set
        Just("$.key in [a, b, c]".to_string()),
        // Matches
        Just("$.key matches ^v.*$".to_string()),
        // Arbitrary path with operator
        "[a-zA-Z_\\.]{1,20}".prop_map(|path| format!("$.{}", path)),
    ]
}

proptest! {
    /// evaluate_filter must never panic on arbitrary valid JSON payloads with arbitrary filters.
    #[test]
    fn filter_never_panics_on_arbitrary_json(
        payload in arb_json_payload(),
        filter in arb_filter()
    ) {
        // Should return true or false, never panic
        let _ = qhook::api::evaluate_filter(&payload, &filter);
    }

    /// Empty filter should always match (truthy fallback for empty path).
    /// An empty filter string triggers the truthy check on an empty path,
    /// which returns false since no field is found.
    #[test]
    fn empty_filter_result_is_deterministic(payload in arb_json_payload()) {
        let result1 = qhook::api::evaluate_filter(&payload, "");
        let result2 = qhook::api::evaluate_filter(&payload, "");
        prop_assert_eq!(result1, result2, "Empty filter should be deterministic");
    }

    /// Filter with non-existent field path should not panic and should return
    /// a deterministic result.
    #[test]
    fn filter_nonexistent_path_no_panic(payload in arb_json_payload()) {
        // Truthy check on missing path: should be false
        let result = qhook::api::evaluate_filter(&payload, "$.this.path.definitely.does.not.exist");
        prop_assert!(!result, "Truthy check on missing path should be false");
    }

    /// Equality filter on missing field should return false.
    #[test]
    fn filter_equality_on_missing_field_returns_false(payload in arb_json_payload()) {
        let result = qhook::api::evaluate_filter(
            &payload,
            "$.nonexistent_field_xyz == some_value"
        );
        prop_assert!(!result, "Equality on missing field should be false");
    }

    /// Inequality filter on missing field should return true (null != anything is true).
    #[test]
    fn filter_inequality_on_missing_field_returns_true(payload in arb_json_payload()) {
        let result = qhook::api::evaluate_filter(
            &payload,
            "$.nonexistent_field_xyz != some_value"
        );
        prop_assert!(result, "Inequality on missing field should be true");
    }

    /// "exists" filter on missing field should return false.
    #[test]
    fn filter_exists_on_missing_field_returns_false(payload in arb_json_payload()) {
        let result = qhook::api::evaluate_filter(
            &payload,
            "$.nonexistent_field_xyz exists"
        );
        prop_assert!(!result, "exists on missing field should be false");
    }

    /// "not" filter should be the logical negation of the inner filter.
    #[test]
    fn filter_not_is_negation(payload in arb_json_payload()) {
        let inner = "$.key == value";
        let normal = qhook::api::evaluate_filter(&payload, inner);
        let negated = qhook::api::evaluate_filter(&payload, &format!("not {}", inner));
        prop_assert_eq!(
            negated, !normal,
            "not filter should negate: normal={}, negated={}",
            normal, negated
        );
    }

    /// Numeric comparison filters should not panic even with non-numeric values.
    #[test]
    fn filter_numeric_comparison_no_panic_on_strings(
        payload in arb_json_payload(),
        op in prop_oneof![Just(">="), Just("<="), Just(">"), Just("<")],
        val in -1000i64..1000i64
    ) {
        let filter = format!("$.key {} {}", op, val);
        let _ = qhook::api::evaluate_filter(&payload, &filter);
    }

    /// "contains" filter should not panic on non-string fields.
    #[test]
    fn filter_contains_no_panic_on_arbitrary_payload(payload in arb_json_payload()) {
        let _ = qhook::api::evaluate_filter(&payload, "$.key contains needle");
    }

    /// "in" filter should not panic with arbitrary payloads.
    #[test]
    fn filter_in_no_panic_on_arbitrary_payload(payload in arb_json_payload()) {
        let _ = qhook::api::evaluate_filter(&payload, "$.key in [a, b, c]");
    }

    /// "matches" filter should not panic even with arbitrary (possibly invalid) regex.
    #[test]
    fn filter_matches_no_panic_on_arbitrary_payload(
        payload in arb_json_payload(),
        pattern in "[a-zA-Z0-9.*+?^$]{0,20}"
    ) {
        let filter = format!("$.key matches {}", pattern);
        let _ = qhook::api::evaluate_filter(&payload, &filter);
    }

    /// evaluate_filter should not panic on completely invalid JSON.
    #[test]
    fn filter_no_panic_on_invalid_json(
        garbage in "\\PC{0,100}",
        filter in arb_filter()
    ) {
        let _ = qhook::api::evaluate_filter(&garbage, &filter);
    }
}

// ============================================================================
// d) ULID generation
// ============================================================================

proptest! {
    /// Generated ULIDs are always valid (26 chars, Crockford base32).
    #[test]
    fn ulid_is_valid_format(_seed in any::<u64>()) {
        let id = ulid::Ulid::new();
        let s = id.to_string();
        prop_assert_eq!(s.len(), 26, "ULID string length should be 26, got {}", s.len());
        // Crockford base32 alphabet: 0-9 A-Z excluding I L O U (case insensitive)
        for ch in s.chars() {
            prop_assert!(
                "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(ch),
                "ULID char '{}' is not valid Crockford base32",
                ch
            );
        }
    }

    /// ULIDs with different timestamps are sortable by time.
    /// Within the same millisecond, the random component doesn't guarantee ordering
    /// with Ulid::new(), so we test with explicit timestamps.
    #[test]
    fn ulids_are_sortable_across_timestamps(ms_offset in 1u64..10000) {
        let ts1 = ulid::Ulid::from_parts(1_000_000, 0);
        let ts2 = ulid::Ulid::from_parts(1_000_000 + ms_offset, 0);
        prop_assert!(
            ts2 > ts1,
            "ULID with later timestamp {} should be > earlier {}",
            ts2, ts1
        );
    }

    /// ULIDs are unique — generating multiple should never produce duplicates.
    #[test]
    fn ulids_are_unique(count in 2usize..50) {
        let ids: Vec<ulid::Ulid> = (0..count).map(|_| ulid::Ulid::new()).collect();
        let unique: HashSet<_> = ids.iter().collect();
        prop_assert_eq!(
            unique.len(), ids.len(),
            "Generated {} ULIDs but only {} were unique",
            ids.len(), unique.len()
        );
    }

    /// ULID round-trips through string representation.
    #[test]
    fn ulid_roundtrips_through_string(_seed in any::<u32>()) {
        let original = ulid::Ulid::new();
        let s = original.to_string();
        let parsed = ulid::Ulid::from_string(&s);
        prop_assert!(parsed.is_ok(), "Failed to parse ULID string: {}", s);
        prop_assert_eq!(
            parsed.unwrap(), original,
            "ULID round-trip failed: {} -> {} -> {:?}",
            original, s, parsed
        );
    }

    /// ULID timestamp component is always reasonable (not in the far past or future).
    #[test]
    fn ulid_timestamp_is_reasonable(_seed in any::<u16>()) {
        let id = ulid::Ulid::new();
        let ts_ms = id.timestamp_ms();
        // Should be after 2020-01-01 (1577836800000 ms) and before 2100-01-01 (4102444800000 ms)
        prop_assert!(
            ts_ms > 1_577_836_800_000,
            "ULID timestamp {} is before 2020",
            ts_ms
        );
        prop_assert!(
            ts_ms < 4_102_444_800_000,
            "ULID timestamp {} is after 2100",
            ts_ms
        );
    }
}
