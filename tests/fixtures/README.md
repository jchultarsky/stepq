# Test fixtures

This directory is populated by `tools/fetch-fixtures.sh` and is ignored
by git except for this file and `manifest.toml`.

| Set | Source | Terms |
|---|---|---|
| `steptools/as1-*-214.stp` | https://steptools.com/docs/stpfiles/ap214/ | Not stated — fetched, not vendored |
| `nist/NIST-PMI-STEP-Files/` | https://www.nist.gov/document/nist-pmi-step-files ([project page](https://www.nist.gov/ctl/smart-connected-systems-division/smart-connected-manufacturing-systems-group/mbe-pmi-0)) | US-Government work; "can be used without any restrictions" |
| `nist-edm/*.stp` | https://github.com/usnistgov/engineering-design-models (`models/STEP/STI`) | Published by NIST as public domain (17 U.S.C. §105); some files originate from STEPnet/industry |
| `ocp/OCP_v3_*.stp`, `ocp/ORv3_*/` | Open Rack V3 reference CAD, Google Drive shares linked from the [Open Compute Project Open Rack wiki](https://web.archive.org/web/20231207045842/https://www.opencompute.org/wiki/Open_Rack/SpecsAndDesigns) | Not stated — fetched, not vendored |
| `ocp/ProjectOlympus*/` | https://github.com/opencomputeproject/Project_Olympus (`HW/`) | © 2016 Microsoft, under the Open Web Foundation Final Specification Agreement 1.0, which does not grant redistribution of the files — fetched, not vendored |

`tools/fetch-fixtures.sh` takes the sets to fetch: `core` (STEP Tools and
NIST, ~20 MB), `ocp` (~130 MB download, ~860 MB unpacked) or `all` (the
default). Every download is checked against a pinned SHA-256.

The `ocp` set is ten Creo exports, from 7 MB and 135 thousand instances to
207 MB and 3.5 million: real-world nested assemblies with hundreds of
products, the scale the other sets lack.

Not fetched, but worth knowing about:

- **NIST MTC box assembly** (`https://www.nist.gov/document/nist-cad-models-mtc-assembly`, ~16 MB)
  is the only modern NIST assembly, but the page describes native NX and
  SolidWorks models; whether it contains STEP is unconfirmed.
- **NIST STC PMI v4** (`https://www.nist.gov/document/nist-stc-pmi-v4`, ~51 MB):
  more AP242 PMI parts, large.
- No public-domain AP242 *assembly* has been found yet.

Tests that need a fixture must skip (not fail) when it is absent, so that
`cargo test` works on a fresh clone.
