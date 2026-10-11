# stepq — notes for AI-assisted sessions

Read `docs/ARCHITECTURE.md` first. It is short and every constraint in it
was learned by breaking something.

Hard rules:
- Read what the file states; never compute what it does not. Reading a
  point, direction, radius, axis, placement or unit factor is in scope, as
  are normalising a direction, applying the stated unit factor of the
  value's own context and composing stated placements. Anything that
  evaluates a curve or surface (intersection, projection, trimmed area,
  volume, tessellation, healing) does not belong here.
- Never guess: a value that cannot be read with certainty is an `Other`
  variant or an error with a reason, never a default or a silent skip.
- No `unsafe`. The crate forbids it.
- Unchanged entities are written verbatim from source text.
- Every closure/extraction change must keep the OCCT volume invariant
  (`tools/verify-occt.py`) green on the AS1 and NIST fixtures, and every
  reader change must keep its `tools/verify-read.py` check green on the
  core fixtures.

Before proposing a PR run: `cargo fmt --check`,
`cargo clippy --all-targets --all-features -- -D warnings`,
`cargo clippy --no-default-features -- -D warnings`,
`cargo test --all-features`, `cargo deny check`,
`cargo +1.85 check --all-features`.

Fixtures: `tools/fetch-fixtures.sh`. Do not vendor STEP files into git.
