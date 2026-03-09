# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in qhook, please report it responsibly.

**Email:** [security@totte.dev](mailto:security@totte.dev)

Please include:

- Description of the vulnerability
- Steps to reproduce
- Affected versions
- Potential impact

We will acknowledge your report within 48 hours and aim to provide a fix within 7 days for critical issues.

**Please do not open a public GitHub issue for security vulnerabilities.**

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.2.x   | Yes       |
| < 0.2   | No        |

## Security Features

qhook includes several built-in security features:

- **Webhook signature verification** — Stripe, GitHub, Shopify, PagerDuty, Grafana, Terraform Cloud, GitLab, SNS X.509, generic HMAC
- **SSRF protection** — Private/loopback IP addresses rejected by default
- **Rate limiting** — Per-IP and per-handler rate limiting
- **Request size limits** — Configurable body size limit (default 1MB)
- **Authentication** — Bearer token for event ingestion and metrics endpoints
- **Security headers** — `X-Content-Type-Options`, `X-Frame-Options`, `Cache-Control`

See the [Security Guide](https://totte-dev.github.io/qhook/guides/security) for configuration details.
