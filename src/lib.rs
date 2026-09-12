//! # stepq
//!
//! Query, inspect, split and reshape STEP (ISO 10303-21) files.
//!
//! `stepq` works on the *entity graph* of a Part 21 file. It never
//! evaluates geometry: there is no tessellation, no volume, no healing.
//! Everything it does — splitting assemblies, extracting a bill of
//! materials, validating structure — is a graph operation over the
//! original entity instances, which are preserved byte-for-byte.
//!
//! The crate is a library first. The `stepq` command-line tool is built
//! from the same code behind the `cli` feature (enabled by default).
//! Library users can depend on it with `default-features = false`.
//!
//! ## Status
//!
//! Pre-1.0. The public API will change. See `ROADMAP.md` in the
//! repository for what is planned and `docs/ARCHITECTURE.md` for the
//! design decisions this crate is built on.

#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod error;
pub mod model;
pub mod p21;

pub use error::{Error, Result};
