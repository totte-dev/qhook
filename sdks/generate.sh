#!/bin/bash
# Generate SDK clients from OpenAPI spec using openapi-generator-cli.
# Usage: bash sdks/generate.sh [python|go|typescript|all]
#
# Prerequisites:
#   npm install -g @openapitools/openapi-generator-cli
#   or: brew install openapi-generator
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SPEC="$SCRIPT_DIR/../docs/openapi.yaml"
VERSION=$(grep 'version:' "$SPEC" | head -1 | sed 's/.*: *//')

generate() {
    local lang=$1
    local generator=$2
    local output="$SCRIPT_DIR/$lang"

    echo "Generating $lang SDK (v$VERSION)..."
    rm -rf "$output"

    openapi-generator-cli generate \
        -i "$SPEC" \
        -g "$generator" \
        -o "$output" \
        --additional-properties=packageName=qhook,packageVersion="$VERSION" \
        --skip-validate-spec

    echo "  -> $output"
}

case "${1:-all}" in
    python)
        generate python python
        ;;
    go)
        generate go go
        ;;
    typescript|ts)
        generate typescript typescript-fetch
        ;;
    all)
        generate python python
        generate go go
        generate typescript typescript-fetch
        ;;
    *)
        echo "Usage: $0 [python|go|typescript|all]"
        exit 1
        ;;
esac

echo "Done."
