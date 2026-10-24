# Fixture scan (no confidential log material)

Pasted logs are confidential: samples and fixtures are invented, never copied. `scripts/ci/scan-fixtures.mjs` (run in CI,
job `licences`) scans `samples/` and every `crates/*/tests/fixtures/` and fails on material that looks real:

- e-mail addresses whose domain is not reserved (`example.*`, `*.test`, `*.invalid`, `localhost`, `*.local`);
- public IPv4 addresses (private, loopback, link-local, CGNAT, documentation ranges `192.0.2.0/24`, `198.51.100.0/24`,
  `203.0.113.0/24`, masks and values after `version`/`build`/`v` are fine);
- lower-case hostnames under company-internal suffixes (`.corp`, `.lan`, `.intranet`, `.internal`, `.home`).

It is conservative on purpose (public domains, user names and GUIDs are left to review). Binary files, archives and files
over 4 MB are skipped.

## Allow list

A reviewed false positive goes into `scripts/ci/scan-fixtures.allow.json` (`values`, matched case-insensitively, with the
reason in `notes`); never loosen a pattern. Current entries: a Java package name, a library version, a synthetic address in
the generated HAR sample and the address example.com resolves to.

Fix a real finding by replacing the value with a documentation-range one (`192.0.2.x`, `user@example.com`,
`host.example.test`) and regenerating the sample (`cargo xtask gen-samples`).
