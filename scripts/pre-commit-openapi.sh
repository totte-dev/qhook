#!/bin/bash
# Pre-commit hook: regenerate SDKs when openapi.yaml changes.
# Install: ln -sf ../../scripts/pre-commit-openapi.sh .git/hooks/pre-commit
#
# Prerequisites: openapi-generator-cli (npm, brew, or npx fallback)
# If the generator is not installed, the hook warns and exits 0 (non-blocking).
set -e

# Only run if openapi.yaml is staged
if ! git diff --cached --name-only | grep -q '^docs/openapi.yaml$'; then
    exit 0
fi

echo "[pre-commit] openapi.yaml changed — regenerating SDKs..."

# Check if generator is available
if ! command -v openapi-generator-cli &>/dev/null && \
   ! command -v openapi-generator &>/dev/null && \
   ! command -v npx &>/dev/null; then
    echo "[pre-commit] WARNING: openapi-generator-cli not found. Skipping SDK regeneration."
    echo "  Install: npm install -g @openapitools/openapi-generator-cli"
    exit 0
fi

# Validate the spec first
SPEC="docs/openapi.yaml"
if command -v openapi-generator-cli &>/dev/null; then
    GENERATOR="openapi-generator-cli"
elif command -v openapi-generator &>/dev/null; then
    GENERATOR="openapi-generator"
else
    GENERATOR="npx --yes @openapitools/openapi-generator-cli"
fi

echo "[pre-commit] Validating OpenAPI spec..."
if ! $GENERATOR validate -i "$SPEC" --recommend 2>/dev/null; then
    echo "[pre-commit] ERROR: OpenAPI spec validation failed. Fix docs/openapi.yaml before committing."
    exit 1
fi

# Regenerate SDKs
echo "[pre-commit] Generating TypeScript and Python SDKs..."
bash sdks/generate.sh all

# Stage regenerated SDK files
git add sdks/typescript/ sdks/python/

echo "[pre-commit] SDKs regenerated and staged."

# Smoke test: verify generated code has no syntax errors
if command -v node &>/dev/null && [ -f sdks/typescript/index.ts ]; then
    echo "[pre-commit] Smoke testing TypeScript SDK..."
    if ! npx --yes tsc --noEmit --strict sdks/typescript/index.ts 2>/dev/null; then
        echo "[pre-commit] WARNING: TypeScript SDK has type errors (non-blocking)."
    fi
fi

if command -v python3 &>/dev/null && [ -f sdks/python/qhook_client/__init__.py ]; then
    echo "[pre-commit] Smoke testing Python SDK..."
    if ! python3 -c "import ast; ast.parse(open('sdks/python/qhook_client/__init__.py').read())" 2>/dev/null; then
        echo "[pre-commit] WARNING: Python SDK has syntax errors (non-blocking)."
    fi
fi

echo "[pre-commit] Done."
