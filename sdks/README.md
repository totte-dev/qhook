# qhook SDKs

Auto-generated client libraries from the [OpenAPI spec](../docs/openapi.yaml).

## Generate

```bash
# All languages
bash sdks/generate.sh all

# Single language
bash sdks/generate.sh python
bash sdks/generate.sh go
bash sdks/generate.sh typescript
```

Requires [openapi-generator-cli](https://openapi-generator.tech/docs/installation):

```bash
npm install -g @openapitools/openapi-generator-cli
# or
brew install openapi-generator
```

## Languages

| Language | Generator | Output |
|----------|-----------|--------|
| Python | `python` | `sdks/python/` |
| Go | `go` | `sdks/go/` |
| TypeScript | `typescript-fetch` | `sdks/typescript/` |

## Usage

Generated SDKs are not published to package registries yet. Use them locally or vendor into your project.

The generated code tracks the OpenAPI spec version — regenerate after spec changes.
