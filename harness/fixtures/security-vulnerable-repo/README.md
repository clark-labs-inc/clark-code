# Clark Security adversarial fixture

This directory is intentionally vulnerable test data. It is not an example
application and must never be deployed, copied into production, or populated
with real credentials.

The fixture mixes TypeScript and Python services behind one fictional
multi-tenant API. It contains deliberately reachable vulnerabilities together
with safe controls that look similar enough to challenge shallow scanners.
The expected findings live outside this fake repository in
`harness/security-vulnerable-oracle.json`.
