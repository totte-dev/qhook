#!/bin/bash
# Generate SDK clients from OpenAPI spec using openapi-generator-cli.
# Usage: bash sdks/generate.sh [python|typescript|all]
#
# Prerequisites:
#   npm install -g @openapitools/openapi-generator-cli
#   or: npx @openapitools/openapi-generator-cli (used automatically)
#   or: brew install openapi-generator
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SPEC="$SCRIPT_DIR/../docs/openapi.yaml"
VERSION=$(grep '  version:' "$SPEC" | head -1 | sed 's/.*: *//')

# Prefer global install, fall back to npx
if command -v openapi-generator-cli &>/dev/null; then
    GENERATOR="openapi-generator-cli"
elif command -v openapi-generator &>/dev/null; then
    GENERATOR="openapi-generator"
else
    GENERATOR="npx --yes @openapitools/openapi-generator-cli"
fi

generate_typescript() {
    local output="$SCRIPT_DIR/typescript"
    echo "Generating TypeScript SDK (v$VERSION)..."
    rm -rf "$output"

    $GENERATOR generate \
        -i "$SPEC" \
        -g typescript-fetch \
        -o "$output" \
        --additional-properties=npmName=qhook-client,npmVersion="$VERSION",supportsES6=true,typescriptThreePlus=true \
        --skip-validate-spec

    # Add .npmignore
    cat > "$output/.npmignore" << 'NPMEOF'
.openapi-generator/
.openapi-generator-ignore
git_push.sh
NPMEOF

    echo "  -> $output"
}

generate_python() {
    local output="$SCRIPT_DIR/python"
    echo "Generating Python SDK (v$VERSION)..."
    rm -rf "$output"

    $GENERATOR generate \
        -i "$SPEC" \
        -g python \
        -o "$output" \
        --additional-properties=packageName=qhook_client,packageVersion="$VERSION",projectName=qhook-client \
        --skip-validate-spec

    echo "  -> $output"
}

case "${1:-all}" in
    python)
        generate_python
        ;;
    typescript|ts)
        generate_typescript
        ;;
    all)
        generate_typescript
        generate_python
        ;;
    *)
        echo "Usage: $0 [python|typescript|all]"
        exit 1
        ;;
esac

echo "Done. SDK version: $VERSION"
