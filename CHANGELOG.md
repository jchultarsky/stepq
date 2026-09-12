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

### Changed

- License is now MIT only (previously MIT OR Apache-2.0).

[Unreleased]: https://github.com/jchultarsky/stepq/compare/main...HEAD
