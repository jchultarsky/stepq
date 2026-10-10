# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Until 1.0, minor versions may contain breaking changes.

## [Unreleased]

The read milestone begins, under a reworded rule: stepq reads what a file
states and never computes what it does not. First, unit factors; then the
geometry values they scale; then the placements that put components into
assemblies; then the topology of a solid; then the solids of a
definition.

### Added

- `stepq::units`: the unit factors a file states. `units::unit` reads any
  SI or conversion-based unit of length, plane angle or solid angle, and
  `units::ContextUnits` what a representation context assigns — its length
  unit to millimetres, plane angle to radians and solid angle to
  steradians, each read or refused on its own, its uncertainties and its
  coordinate space dimension — for a context, a representation, or a
  solid, shell or face, whose holding contexts must agree (a holder stepq
  cannot follow is reported, never stepped over). Factors are
  used as stated (a degree of `0.0174532925` radian stays that) and nothing
  is assumed: what cannot be read is a `units::UnitError` with the reason,
  and `stepq::Error` gains a `Unit` variant. On all 55 fixtures, the 1,670
  contexts' factors and uncertainties equal Open CASCADE 8.0.1's, bit for
  bit but one.
- `tools/verify-read.py units` compares every unit context with Open
  CASCADE's reading (through `examples/units.rs`), and
  `tools/gen-occt-cases.py` writes the same plate in five length units for
  it. CI runs both, and the fixture tests with
  `STEPQ_REQUIRE_FIXTURES=1`, which fails them if a core fixture is
  missing.
- A `read` fuzz target, which runs every reader on every instance. CI starts
  it from hand-written seeds (`fuzz/seeds/read`) that reach every reader.
- `stepq::geometry`: the geometry a file states, read by `#id` in
  millimetres and radians. A `geometry::Reader`, tied to a representation or
  to a solid, shell or face whose holders must agree, reads points,
  directions (normalised, never scaled), vectors, axis placements (the
  schema's `build_axes`, saying whether the axis and `ref_direction` were
  stated), planes, cylindrical, conical, spherical and toroidal surfaces,
  lines, circles and surface curves (their `curve_3d` and
  `master_representation`). Every other entity and every complex instance
  is an `Other` naming its type; what cannot be read as stated is a
  `geometry::GeometryError` naming the innermost instance, the attribute
  and why, and `stepq::Error` gains a `Geometry` variant. `geometry::Rigid`
  is the transform placements will compose. Two policies bound what is
  read: a `ref_direction` within a sine of 1e-6 of its axis, and a cone
  semi-angle within 1e-6 rad of 0 or π/2, are refused. A radius must be
  positive in millimetres as well as written, so one that a tiny unit takes
  to zero is refused, and a length that is not finite once converted is
  refused. On all 55 fixtures every face surface (342,733) and edge curve
  (979,505) reads, and `tools/verify-read.py surfaces` and `curves` match
  the 331,816 elementary surfaces and 905,916 lines and circles of their
  shape representations with Open CASCADE 8.0.1's one to one, to 5.6e-16.
  CI runs both on the core fixtures and on a part `tools/gen-occt-cases.py`
  writes with every elementary surface, a cone point and a void, in
  millimetres and inches. `curves` excuses the seams and split edges Open
  CASCADE adds while it reads, so an edge stepq misses that is a seam or
  equals another edge shows only in its healing tallies, which CI pins
  (`--healing`).
- `model::Placements`: the transform each assembly usage states, in
  millimetres, and every occurrence under a root, with its path of usages,
  the representation it maps from and its transform to the root, composed
  from the root down by a lazy iterator that holds only the current path.
  Each frame is read in the context its position assigns (ISO 10303-43). An
  `item_defined_transformation` is `M(assembly-side item) ·
  M(component-side item)⁻¹`, the usage deciding which side is the
  component's, so the 131 reversed Pro/ENGINEER usages of the fixtures are
  placed right; a mapped item is `M(target) · M(origin)⁻¹`. Nothing is
  assumed: a usage that is unplaced, quantified other than once, placed by a
  relationship with no transformation, a transformation operator or a frame
  that is not an `axis2_placement_3d`, structurally inconsistent, or one that
  ISO 10303 and Open CASCADE place differently (both transform items not the
  identity, or a frame without `ref_direction` off +Z) is a
  `model::PlacementError` for it and every occurrence below it, and
  `stepq::Error` gains a `Placement` variant. On all 55 fixtures the 3,481
  usages and 4,359 occurrences read, and `tools/verify-read.py occurrences`
  matches every occurrence with Open CASCADE 8.0.1's to 2.2e-16 in rotation
  and 1.3e-13 mm in translation. CI runs it on the core fixtures and on a
  nested assembly Open CASCADE writes, with two divergence cases made from
  it that must be refused (`tools/gen-occt-cases.py`), and pins the counts.
  `examples/occurrences.rs` prints stepq's side.
- `stepq::topology`: the B-rep topology of one solid as stated.
  `topology::Solid::read` reads a `manifold_solid_brep` or `brep_with_voids`
  whole: its shells (an `oriented_closed_shell` as outer shell or void),
  `advanced_face`s, bounds, edge and vertex loops, oriented edges, edge
  curves and vertex points, by index with each `#id` kept, every flag as
  written. It also reads the vertices each oriented edge derives, every use
  of each edge, each face's sense in the solid (`face_sense`) and each use's
  direction (`forward`), both composed through an `oriented_closed_shell` as
  the schema derives its faces, so a hole reads as a hole in a void as in
  the outer shell. Valid STEP it does not read (a `faceted_brep`, a surface
  model, a `face_surface`, AP214's `seam_edge`, any complex instance) is
  `TopologyError::Unsupported`, as are faces that share a bound or loop,
  since each instance is read once and a read stays linear in the file;
  what contradicts the schema (a wrong type or attribute count, an empty
  set, a loop that does not close, a repeated oriented edge, two outer
  bounds, a face held twice) is `Malformed`; each names the innermost
  instance, and `stepq::Error` gains a `Topology` variant. On all 55 fixtures 1,260 solids read (257,358 faces,
  715,015 edges, each used twice in opposite directions), the 87 surface
  models are refused as unsupported, and one Creo part whose loop does not
  close is refused as malformed. `tools/verify-read.py topology` joins every
  solid with Open CASCADE 8.0.1's on entity numbers and compares faces and
  their senses, the faces using each edge, each edge's first vertex, the
  direction of every use and that seams are closed; CI runs it on the core
  fixtures and the generated parts, where all 132 solids match. Open
  CASCADE replaces some edges while healing, which are counted, not
  compared; CI pins their count (`--healing=278`). `tools/gen-occt-cases.py` adds a cube with a cubic void, so that
  uses in a reversed shell are compared, and CI's occurrence pin moves to
  376. `examples/topology.rs` prints stepq's side.
- `ProductStructure::shape`: a product definition's own shape, read from its
  shape representations: its solids (`manifold_solid_brep`,
  `brep_with_voids` and `faceted_brep` items), its other items with their
  types, and every link not followed with the reason, so that an empty,
  complete shape states no geometry and `DefinitionShape::is_complete`
  says when stepq cannot tell. It follows a plain
  `shape_representation_relationship` that no usage names between two
  representations of one context, either way round, and a mapped item that
  is not a component placement; it skips frames, usages' relationships,
  component mapped items and supplemental geometry, each only when the far
  side is what the rule expects. A transformed relationship no usage names,
  any other relationship, one across contexts, a representation another
  definition reaches too, a cycle, a walk too large, a malformed reference
  and a shape in another file (an `applied_document_reference`, an
  'external definition', as `split --master` writes stubs) are reported.
  Each item keeps its path, and `ShapeItem::transform` composes the mapped
  items on it in millimetres. On all 55 fixtures the 1,660 definitions are
  complete and their 1,261 solids are each reached once;
  `tools/verify-read.py shapes` matches every definition's subtree solids
  with Open CASCADE 8.0.1's shape labels, and CI runs it on the core
  fixtures. `examples/shapes.rs` prints stepq's side.

### Changed

- The rule, in the README, `docs/ARCHITECTURE.md`, `ROADMAP.md` and
  `CONTRIBUTING.md`: stepq reads what a file states — a point, an axis, a
  radius, a placement, a unit factor — and never computes what it does not.
  Normalising a direction, applying the stated unit factor of the value's
  own context and composing stated placements count as reading;
  intersections, trimmed areas, tessellation, volumes, bounding boxes and
  healing stay out. A value that cannot be read with certainty is reported,
  never guessed. Output does not change: unchanged entities are still
  written byte for byte.
- `docs/ARCHITECTURE.md`, "Assembly structure": `transform_item_k` is an
  item of `rep_k`, and the transform carries the component-side item onto
  the assembly-side one. It said `transform_item_1` is the child frame,
  which misplaces 131 of the 139 occurrences in the Pro/ENGINEER fixtures.

### Fixed

- `ProductStructure` places a usage through a mapped item written as a
  complex instance: it read the item's `mapping_source` and
  `mapping_target` where a simple instance holds them, one position too
  far, and left such usages unplaced. It also reads the items of a complex
  representation. No fixture has either, and no output changes on them.

## [0.4.1] - 2026-09-17

Wording fixes in `split`'s output and help.

### Fixed

- `split` says "wrote 1 file" rather than "wrote 1 files".
- `split --help` describes the two groups `--report-orphans` lists (left
  behind, referenced by nothing) and the JSON `unreferenced` field.

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

[Unreleased]: https://github.com/jchultarsky/stepq/compare/v0.4.1...HEAD
[0.4.1]: https://github.com/jchultarsky/stepq/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/jchultarsky/stepq/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/jchultarsky/stepq/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/jchultarsky/stepq/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jchultarsky/stepq/releases/tag/v0.1.0
