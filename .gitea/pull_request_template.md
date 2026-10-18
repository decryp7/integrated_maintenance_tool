## What and why

<!-- What does this change do for users, and why? Link issues. -->

## How it was tested

<!-- Tests added or changed; manual checks (docs/testing-manual.md sections) on which platforms. -->

## Checklist

- [ ] Title follows Conventional Commits (`feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `ci:`, `chore:`)
- [ ] Tests ship with the change; a bug fix starts with a failing regression test
- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny --workspace check` pass
- [ ] `npx ng build`, `npx ng test --watch=false` and `npx ng lint` pass (in `app/`)
- [ ] Wire contract changes are mirrored in `api.types.ts` and the Rust DTOs
- [ ] New profile keys are optional and documented in `default_profile.toml`
- [ ] Untrusted input is bounded and never panics; new persistent state is versioned
- [ ] Docs / ADR updated where behaviour or architecture changed
