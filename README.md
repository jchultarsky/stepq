# stepq

[![CI](https://github.com/jchultarsky/stepq/actions/workflows/ci.yml/badge.svg)](https://github.com/jchultarsky/stepq/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/stepq.svg)](https://crates.io/crates/stepq)
[![docs.rs](https://docs.rs/stepq/badge.svg)](https://docs.rs/stepq)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-orange.svg)](Cargo.toml)

Query, inspect, split and reshape STEP (ISO 10303-21) files — a Rust library
and a command-line tool.

`stepq` works on the **entity graph** of a STEP file, not on its geometry.
It never tessellates, never computes a volume, never "heals" anything.
That constraint is the point: every entity that describes your geometry
comes out exactly as it went in, byte for byte, with its original names,
colours, layers, properties and PMI attached.

> **Status: 0.1, early.** The Part 21 parser, reference graph, writer and
> EXPRESS schema reader are in place, and `stepq info`, `tree`, `bom` and
> `split` work. The library API and the CLI output will still change
> before 1.0; see [ROADMAP.md](ROADMAP.md).

## What it does

```console
$ stepq info  assembly.stp                # header, schema, units, entity histogram
$ stepq tree  assembly.stp --usages       # assembly hierarchy and placements
$ stepq bom   assembly.stp                # multi-level bill of materials as a tree
$ stepq bom   assembly.stp --format csv   # the same as an indented CSV
$ stepq split assembly.stp --out parts/   # one file per sub-assembly and part
```

Every command reads `-` as standard input and prints a table, JSON
(`--format json`) or CSV (`--format csv`).

```console
$ stepq bom as1-ac-214.stp
AS1-AC-214
├── PLATE
├── L-BRACKET ASSEMBLY  ×2
│   ├── L-BRACKET  (2 total)
│   └── NUT-BOLT ASSEMBLY  ×3  (6 total)
│       ├── BOLT  (6 total)
│       └── NUT  (6 total)
└── ROD-ASSEMBLY
    ├── ROD
    └── NUT  ×2

3 sub-assemblies, 5 distinct parts, 18 parts in total
```

The headline feature is `split`: explode an assembly into self-contained
STEP files for every sub-assembly and part, preserving nested structure,
instance placement, colours and names — without a CAD seat and without
regenerating a single surface. As far as we can tell nothing open-source
does this today; the usual answer is "open it in SolidWorks and Save As".

Planned beyond that: `lint` (structural problems, no kernel needed),
`query` and `refs` (jq-style entity search and
back-reference lookup), structural `diff` of two files, `props` and `pmi`
extraction to JSON, `strip`/`anonymize`, and `assemble` (the inverse of
`split`). See [ROADMAP.md](ROADMAP.md) for the tiers.

## What it will not do

Anything geometric. No tessellation, no mass properties, no bounding boxes,
no unit *conversion*, no shape healing, no format conversion. If you need
those, you need a geometry kernel; [Open CASCADE](https://dev.opencascade.org/)
is the open-source one. `stepq` is designed to sit next to a kernel, not
replace it.

## Install

Prebuilt binaries for macOS, Linux and Windows are attached to every
[GitHub release](https://github.com/jchultarsky/stepq/releases). The
installers put `stepq` in `~/.cargo/bin` without needing Rust.

macOS and Linux:

```console
$ curl --proto '=https' --tlsv1.2 -LsSf https://github.com/jchultarsky/stepq/releases/latest/download/stepq-installer.sh | sh
```

Windows (PowerShell):

```console
> powershell -ExecutionPolicy Bypass -c "irm https://github.com/jchultarsky/stepq/releases/latest/download/stepq-installer.ps1 | iex"
```

The installers also install `stepq-update`, which upgrades to the latest
release. With a Rust toolchain (1.85 or newer) instead:

```console
$ cargo install stepq            # or: cargo binstall stepq
```

As a library, without the CLI dependencies:

```toml
[dependencies]
stepq = { version = "0.1", default-features = false }
```

## Supported input

ISO 10303-21 editions 1 and 2 (`implementation_level '2;1'`), which is what
every mainstream CAD system writes. Application protocols AP203, AP214 and
AP242 (all editions). Part 21 edition 3 features (anchors, references,
multiple data sections) are parsed leniently but not interpreted.

## Design

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before contributing. The
short version:

* **Back-references first.** In STEP, a product's shape, colours, PMI and
  placement all point *at* the product. A forward closure from a
  `product_definition` reaches six entities and no geometry.
* **Untyped entities, schema-checked.** AP242 has ~2,400 entity types; real
  files use ~230. We keep instances as name + attribute tokens and validate
  attribute counts against the EXPRESS schema rather than generating a
  struct per type.
* **Verbatim output.** Unchanged entities are written back from their
  original text. Only `#id`s are renumbered.
* **Silent failure is the enemy.** A dropped reverse reference produces a
  file every tool reads happily with zero solids and no warning. The test
  oracle is therefore geometric: every output is read back through Open
  CASCADE and checked against volume and solid-count invariants of the
  input (`tools/verify-occt.py`). CI already uses it to check that
  rewriting every sample file through stepq changes no geometry.

## Contributing

Issues and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md)
for how the project is organised, how to get the test fixtures, and what CI
checks. Everyone taking part is expected to follow the
[Code of Conduct](CODE_OF_CONDUCT.md).

Found a crash or hang on a crafted file? Please report it privately — see
[SECURITY.md](SECURITY.md).

## License

`stepq` is released under the [MIT License](LICENSE).

Unless you explicitly state otherwise, any contribution you intentionally
submit for inclusion in this project is licensed under the MIT License,
without any additional terms or conditions.
