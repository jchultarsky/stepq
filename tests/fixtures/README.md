# Test fixtures

This directory is populated by `tools/fetch-fixtures.sh` and is ignored
by git except for this file and `manifest.toml`.

| Set | Source | Terms |
|---|---|---|
| `steptools/as1-*-214.stp` | https://steptools.com/docs/stpfiles/ap214/ | Not stated — fetched, not vendored |
| `nist/NIST-PMI-STEP-Files/` | https://www.nist.gov/document/nist-pmi-step-files | US-Government work, public domain |

Tests that need a fixture must skip (not fail) when it is absent, so that
`cargo test` works on a fresh clone.
