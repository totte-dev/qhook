#!/usr/bin/env bash
# check-docs.sh — Verify documentation stays in sync with source code.
# Run in CI to catch missing doc updates before merge.
# Works on both macOS (BSD grep) and Linux (GNU grep).
set -euo pipefail

ERRORS=0

err() {
  echo "ERROR: $1"
  ERRORS=$((ERRORS + 1))
}

warn() {
  echo "WARN:  $1"
}

# ─── 1. CLI subcommands ───
echo "=== Checking CLI commands ==="

# Extract doc comments (/// lines) from cli.rs that describe subcommands
for cmd in $(sed -n 's/^[[:space:]]*\/\/\/[[:space:]]*\(List\|Replay\|Retry\|Redrive\|Start\|Init\|Validate\).*/\1/p' src/cli.rs | sort -u); do
  lower=$(echo "$cmd" | tr '[:upper:]' '[:lower:]')
  if ! grep -qi "$lower" docs/cli.md 2>/dev/null; then
    err "CLI command '$cmd' found in src/cli.rs but not documented in docs/cli.md"
  fi
done

# Check top-level subcommands from enum Command
for cmd in $(grep -Eo '(Start|Init|Validate|Jobs|Events|WorkflowRuns)' src/cli.rs | sort -u); do
  lower=$(echo "$cmd" | tr '[:upper:]' '[:lower:]' | sed 's/workflowruns/workflow-runs/')
  if ! grep -q "qhook $lower" docs/cli.md 2>/dev/null; then
    err "CLI command 'qhook $lower' not documented in docs/cli.md"
  fi
done

# ─── 2. Prometheus metrics ───
echo "=== Checking Prometheus metrics ==="

for metric in $(grep -oE 'qhook_[a-z_]+' src/metrics.rs | sort -u); do
  if ! grep -q "$metric" docs/guides/monitoring.md 2>/dev/null; then
    err "Metric '$metric' found in src/metrics.rs but not documented in docs/guides/monitoring.md"
  fi
done

# ─── 3. API endpoints → OpenAPI ───
echo "=== Checking API endpoints ==="

for path in $(grep -oE '"/[a-z_/:{}]+"' src/api.rs | tr -d '"' | sort -u); do
  # Normalize axum :param to OpenAPI {param}
  openapi_path=$(echo "$path" | sed 's/:\([a-z_]*\)/{\1}/g')
  if ! grep -q "$openapi_path" docs/openapi.yaml 2>/dev/null; then
    if [ "$path" != "/healthz" ]; then
      warn "Endpoint '$path' in src/api.rs may not be in docs/openapi.yaml (check: $openapi_path)"
    fi
  fi
done

# ─── 4. CHANGELOG has Unreleased or version entry ───
echo "=== Checking CHANGELOG ==="

if ! head -20 CHANGELOG.md | grep -qE '## \[Unreleased\]|## \[0\.' ; then
  warn "CHANGELOG.md has no [Unreleased] section and no recent version entry in first 20 lines"
fi

# ─── 5. Cargo.toml version vs Chart.yaml appVersion ───
echo "=== Checking version consistency ==="

CARGO_VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
if [ -f charts/qhook/Chart.yaml ]; then
  CHART_APP_VERSION=$(grep '^appVersion' charts/qhook/Chart.yaml | sed 's/.*"\(.*\)"/\1/')
  if [ "$CARGO_VERSION" != "$CHART_APP_VERSION" ]; then
    warn "Cargo.toml version ($CARGO_VERSION) != charts/qhook/Chart.yaml appVersion ($CHART_APP_VERSION)"
  fi
fi

# ─── 6. Environment variables ───
echo "=== Checking environment variables ==="

for var in QHOOK_LOG_FORMAT OTEL_EXPORTER_OTLP_ENDPOINT; do
  if grep -rq "$var" src/ && ! grep -q "$var" docs/cli.md 2>/dev/null; then
    err "Env var '$var' used in source but not documented in docs/cli.md"
  fi
done

# ─── Summary ───
echo ""
if [ $ERRORS -gt 0 ]; then
  echo "FAILED: $ERRORS error(s) found. Update documentation to match source code."
  exit 1
else
  echo "OK: All documentation checks passed."
fi
