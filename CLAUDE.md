# stepq — notes for AI-assisted sessions

Read `docs/ARCHITECTURE.md` first. It is short and every constraint in it
was learned by breaking something.

Hard rules:
- No geometry evaluation, ever. If a change needs a curve or surface
  value, it does not belong here.
- No `unsafe`. The crate forbids it.
- Unchanged entities are written verbatim from source text.
- Every closure/extraction change must keep the OCCT volume invariant
  (`tools/verify-occt.py`) green on the AS1 and NIST fixtures.

Before proposing a PR run: `cargo fmt --check`,
`cargo clippy --all-targets --all-features -- -D warnings`,
`cargo clippy --no-default-features -- -D warnings`,
`cargo test --all-features`, `cargo deny check`.

Fixtures: `tools/fetch-fixtures.sh`. Do not vendor STEP files into git.
