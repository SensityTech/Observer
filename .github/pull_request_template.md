## What

## Why

## Verification

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`
- [ ] `web/`: lint, type check, tests and build
- [ ] UI checked in the browser in both themes (when the panel changed)
- [ ] Platform code checked on Windows and Linux (when `cfg(windows)` or `cfg(unix)` code changed)
