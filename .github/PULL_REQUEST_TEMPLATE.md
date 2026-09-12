## What

<!-- One or two sentences. Link the issue if there is one. -->

## Why

<!-- The reasoning, for the person reading `git blame` later. -->

## Checklist

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo clippy --no-default-features -- -D warnings`
- [ ] `cargo test --all-features`
- [ ] `cargo deny check`
- [ ] Tested against a real STEP file where relevant (say which)
- [ ] Closure/extraction changes: OCCT volume invariant green on AS1 and NIST
- [ ] `CHANGELOG.md` updated under **Unreleased** if user-visible
- [ ] I agree to license this contribution under the MIT License
