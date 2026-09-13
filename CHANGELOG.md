# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Until 1.0, minor versions may contain breaking changes.

## [Unreleased]

### Added

- `stepq lint`: syntax errors, duplicate ids, dangling references,
  transformations whose representations share a context, a missing
  application protocol definition and products with no category; with
  `--schema` (a file or a directory of `.exp` files), also unknown entity
  types, attribute counts and lists below their lower bound. Table (10
  findings per check unless `--all`), JSON or CSV; exits 1 on errors. Also
  available as the library function `lint::lint`.

- Open Compute Project test fixtures: ten Open Rack V3 and Project Olympus
  assemblies (Creo, AP214 and AP203), up to 207 MB and 3.5 million
  instances. `tools/fetch-fixtures.sh` takes the sets to fetch (`core`,
  `ocp`, `all`) and checks every download against a pinned SHA-256.

## [0.1.0] - 2026-09-13

First release.

### Added

- Prebuilt binaries for macOS (arm64, x86-64), Linux (arm64, x86-64 glibc
  and musl) and Windows (x86-64), with shell and PowerShell installers and
  `stepq-update`, built by dist.
- Project skeleton: library crate with feature-gated `stepq` CLI, CI,
  lint and license configuration.
- `p21::Lexer`: zero-copy, non-line-oriented Part 21 tokeniser covering
  edition 2 plus edition 3 names and resource references, with line and
  column in syntax errors.
- `p21::decode_string`: decodes `''`, `\\`, `\X\`, `\X2\`, `\X4\`, `\S\`,
  `\P?\`, `\N\` and `\T\` in string literals.
- `p21::parse`: validates the whole exchange structure (header, one or more
  data sections, simple and complex instances, nested and typed
  parameters) into an `Exchange` of untyped instances, keeping each
  instance's original text span. Views (`Record`, `Params`, `Param`,
  `Literal`) walk parameters on demand.
- `model::Graph`: forward and back reference indices over an `Exchange`.
  `Graph::new` rejects dangling references; `Graph::build` records them in
  `unresolved()` for reporting.
- Query helpers shared by all commands: `Exchange::instances_of`,
  `Exchange::header_entity`, `Record::param`, and `Param::reference`,
  `list`, `typed`, `literal` and `is_unset`.
- `p21::Writer`: writes all instances or a reference-closed selection.
  Instances are copied byte for byte from source; `Numbering::Dense`
  rewrites only `#id` tokens, never text inside strings or comments.
- `stepq info`: header fields, schema, declared units, instance, product
  and assembly-usage counts, dangling references and an entity-type
  histogram, as a table (`--top N`), JSON or CSV. `-` reads standard
  input. The summary is also available as the library type `info::Info`.
- `tools/verify-occt.py`: reads STEP files through Open CASCADE (via uv
  and `cadquery-ocp`) and checks that outputs keep the input's solid count,
  total volume (relative 1e-9), names and colours. `tools/verify-rewrite.sh`
  and a CI job apply it to every fixture rewritten by the new `rewrite`
  example.
- `model::extract` and `model::RULES`: extracts a product definition with
  everything that belongs to it — a fixpoint over forward references and a
  rule table of back references (shapes, placements, styles, PMI, child
  usages). Shared aggregates (layers, categories, approvals, …) are kept
  with their lists filtered. `model::orphans` reports what no extraction
  takes.
- `p21::Writer::write_pruned`: writes a selection in which shared
  aggregates drop list items that are not selected.
- `stepq split`: writes one self-contained file per product definition,
  renumbered, with `--report-orphans` and `--force`.
- `tools/verify-split.py`: splits every fixture and checks through Open
  CASCADE that top-level outputs reproduce the input and that each
  assembly equals its components.
- Fuzz targets `lex` and `parse` (cargo-fuzz, with a Part 21 dictionary),
  run for 60 seconds each in CI.
- `express::Schema`: reads long-form EXPRESS schemas at run time
  (entities, supertypes, explicit attributes, explicit and derived
  redeclarations, defined types) and computes the Part 21 attribute layout
  of simple and complex instances, with aggregate lower bounds.
- `express::check`: reports unknown entity types, wrong attribute counts
  and lists shorter than their aggregate bound.
- `tools/fetch-schemas.sh`: downloads the AP242 ed. 4, AP214 ed. 3 and
  AP203 ed. 1 and 2 schemas, checksum-pinned. Schemas are ISO copyright
  and are not committed or shipped.
- `model::ProductStructure`: product definitions (with product, version
  and shape representations), assembly usages (NAUO and quantified usages,
  simple or complex), and the entities that place each usage, flagging
  shape relationships whose `rep_1`/`rep_2` are reversed. Parent and child
  always come from the usage. Includes a rolled-up bill of materials.
- Placements expressed only through `MAPPED_ITEM` / `REPRESENTATION_MAP`
  are read and matched to their usages when the counts agree.
  `model::Placement` is now an enum: `ShapeRelationship` or `MappedItem`.
- `stepq tree`: the assembly hierarchy with repeated components grouped
  (`--usages` lists each usage and its placement), as a table, JSON or CSV.
- `stepq bom`: the multi-level bill of materials of each top-level
  assembly, with quantity per assembly and total quantity. Drawn as a tree
  (`--charset`, `--prefix`, `--depth`; repeated sub-assemblies expanded
  once and marked `(*)` unless `--no-dedupe`), as an indented CSV with
  level and item number, or as nested JSON. `--flat` gives one line per
  distinct component with its total quantity. The tree is also available
  as `ProductStructure::bom_tree`.
- `--format tree`, which commands without a tree view treat as `table`.

### Changed

- License is now MIT only (previously MIT OR Apache-2.0).

[Unreleased]: https://github.com/jchultarsky/stepq/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jchultarsky/stepq/releases/tag/v0.1.0
