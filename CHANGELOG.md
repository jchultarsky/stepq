# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Until 1.0, minor versions may contain breaking changes.

## [Unreleased]

## [0.4.0] - 2026-09-13

Extraction coverage: the rule table grown from orphan reports on every
fixture, so `split` keeps PMI presentation, saved views, notes, documents
and more with their products; the orphan report tells what is left behind
from what nothing refers to; a command reference (`docs/COMMANDS.md`); and
small CLI output fixes.

### Added

- `split` and `model::extract` keep much more of what belongs to a
  product: PMI presentation (annotation planes, draughting model
  relationships, per-annotation validation properties, callout
  relationships, display formats of PMI values), saved views (AP242
  cameras and Creo presentation sets, areas and views), supplemental
  geometry, notes and polyline or tessellated annotation in a draughting
  model, tolerance zones, composite tolerances, referenced standards
  documents and addresses; `directed_dimensional_location` and
  `default_model_geometric_view` now match. On the fixtures under 16 MB,
  instances in no output went from 43,534 to 1,559. Shared rules can follow
  list items of given types (`model::Rule::follow`).
- `split --report-orphans` lists orphans left behind apart from those
  nothing refers to; JSON adds `unreferenced`. Library:
  `model::unreachable`.

### Changed

- `props --format json` writes the property kind as `user`, the name
  `props --kind` and the table use, instead of `user_defined` (also the
  `serde` serialization of `props::PropertyKind::UserDefined`).

### Fixed

- `split --report-orphans` says "1 instance is in no output" rather than
  "1 instances are".
- `split`, `strip` and `assemble` word a refused overwrite the same way:
  "… already exists; pass --force to overwrite it".
- `split --bodies` checks whether the `<part>.body-<n>.stp` files exist
  before writing anything, instead of stopping part-way through.
- `props` and `diff` tables print a value whose text spans lines (such as
  a centroid point) on one line.
- `props` shows a property with no name as `(unnamed)` in the table.
- `bom --depth` says when its summary line counts levels the tree leaves
  out.
- `pmi` lists datums, then tolerances, then dimensions under each product
  definition, each in instance order, instead of mixing them.
- `tools/verify-occt.py` sums volumes solid by solid. Open CASCADE's volume
  of a compound that also holds open shells (Creo surface models) is not
  the sum of its parts, which failed the assembly check on three Open Rack
  fixtures although every placed solid matched its component to 1e-15.

## [0.3.0] - 2026-09-13

Reshaping and PMI: `split --bodies`, `split --master` and its inverse
`assemble`, semantic GD&T with `pmi`, Part 21 edition 3, and a Homebrew
formula.

### Added

- `stepq assemble MASTER -o OUT`: the inverse of `split --master`. Every
  component stub with a CAx-IF external reference is replaced by the
  instances of the file it names (read relative to the master), renamed
  after the master's; references to the stub are pointed at the
  component's own instances and the reference entities are dropped. Nested
  masters are merged first. Library: `assemble::assemble`, with
  `p21::Numbering::Offset` and `p21::Writer::write_instances`.
- `stepq split --master`: assemblies are written as master files. Each
  component keeps its product, definition, placements and a shape without
  geometry, and refers to its own file through CAx-IF external references
  (Recommended Practices for External References 3.1: `document_file`,
  `applied_external_identification_assignment`,
  `applied_document_reference`). Open CASCADE reads the top master of the
  AS1 assembly from three exporters, nested masters included, with the
  original solid count and volume; it does not attach external files to
  components placed through `mapped_item`s. `p21::Writer::append` writes instances the source does
  not have.
- Part 21 edition 3 anchors and references: `Exchange::anchors`,
  `Exchange::external_references` and `Exchange::is_external`. Instance
  names a `REFERENCE` section defines no longer count as dangling. The
  writer keeps `ANCHOR`, `REFERENCE` and `SIGNATURE` sections verbatim
  when the output is unchanged; when renumbering or selecting, it renames
  anchor and reference entries, drops those that no longer apply, and
  drops the signature.
- `stepq pmi`: semantic GD&T of AP242 files — datums, geometric
  tolerances (characteristic, magnitude, datum reference frame with
  modifiers, tolerance modifiers, unit size, toleranced feature) and size
  and location dimensions (nominal values, plus-minus bounds, features) —
  grouped by product definition, with values as written; table, JSON or
  CSV. Library: `pmi::pmi`.
- `stepq split --bodies`: a part whose shape holds several solids also gets
  one file per solid, `<part>.body-<n>.stp`, holding the part with its
  other solids and everything only they bring (faces, styles) left out.
  Library: `model::extract_excluding`.
- Releases publish a Homebrew formula to `jchultarsky/homebrew-tap`.

## [0.2.0] - 2026-09-13

Inspection commands: `lint`, `refs`, `query`, `props`, `diff` and
`strip`, plus the multi-level `bom`.

### Added

- `stepq strip FILE -o OUT`: blanks the header's author, organization and
  authorization and every string of people, organizations and addresses;
  with `--anonymize`, also renames products and usages and blanks the
  file name, descriptions, usage and shape names and user-defined
  attribute text. Only those strings change; the Open CASCADE check shows
  identical geometry. Refuses to overwrite without `--force`. Also
  available as `strip::strip`, with the new `p21::Replacements` for the
  writer.
- `stepq diff OLD NEW`: compares two files by what they describe, not by
  instance names: header fields, units and instance count, entity-type
  counts, products (matched by `product.id`), component quantities per
  assembly and property values (with `#id`s in labels ignored).
  `--section` selects sections; exits 1 if the files differ; table, JSON
  or CSV. Also available as `diff::diff`.
- `stepq props`: user-defined attributes, geometric validation properties,
  other properties and persistent identifiers (`id_attribute`), grouped by
  the product definition they belong to, with values as written in the
  file; `--kind validation|user|other|id`; table, JSON or CSV. Also
  available as `props::properties`.

- `stepq refs FILE ID...`: what instances refer to and what refers to
  them, `--direction both|out|in`, `--depth` up to 64 with repeats marked
  `(*)`, as a table, nested JSON or CSV.
- `stepq query FILE`: instances by `--type` (repeatable; complex instances
  match on any partial entity) and `--contains`, with `--limit`, `--count`
  and `--full`, as a table, JSON or CSV.

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

### Changed

- `split` and `model::extract` now keep persistent identifiers
  (`id_attribute`) and `description_attribute`s with what they describe;
  every split left them behind before.
- Assembly-scope validation properties need no special handling on
  re-root: an extraction carries only its root's own properties, and in
  every AS1 split output the declared volume matches Open CASCADE
  (`docs/ARCHITECTURE.md`).

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

[Unreleased]: https://github.com/jchultarsky/stepq/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/jchultarsky/stepq/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/jchultarsky/stepq/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/jchultarsky/stepq/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jchultarsky/stepq/releases/tag/v0.1.0
