# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Until 1.0, minor versions may contain breaking changes.

## [Unreleased]

### Added

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

### Changed

- License is now MIT only (previously MIT OR Apache-2.0).

[Unreleased]: https://github.com/jchultarsky/stepq/compare/main...HEAD
