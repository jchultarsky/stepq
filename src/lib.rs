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
//! ## Pipeline
//!
//! Every command is built from the same four steps: [`p21::parse`] the
//! bytes into an [`Exchange`](p21::Exchange), index its references with a
//! [`model::Graph`], ask questions through the record and parameter
//! views, and write a selection back out with a [`p21::Writer`].
//!
//! ```
//! use stepq::model::Graph;
//! use stepq::p21::{Numbering, Writer, parse};
//!
//! let src = b"ISO-10303-21;HEADER;FILE_SCHEMA(('AP242'));ENDSEC;DATA;
//! #1=PRODUCT('p1','bracket','',(#2));
//! #2=PRODUCT_CONTEXT('',#3,'mechanical');
//! #3=APPLICATION_CONTEXT('design');
//! #4=PRODUCT_DEFINITION_FORMATION('','',#1);
//! ENDSEC;END-ISO-10303-21;";
//!
//! let graph = Graph::new(parse(src)?)?;
//! let exchange = graph.exchange();
//!
//! // Find instances by type and read their attributes.
//! let product = exchange.instances_of("PRODUCT").next().unwrap();
//! let record = exchange.records(graph.instance(product)).next().unwrap();
//! let name = record.param(1).and_then(|p| p.literal()).unwrap().decode()?;
//! assert_eq!(name, "bracket");
//!
//! // Follow references backwards: the formation points at the product.
//! let users: Vec<u64> = graph
//!     .referenced_by(product)
//!     .iter()
//!     .map(|&node| graph.instance(node).id)
//!     .collect();
//! assert_eq!(users, [4]);
//!
//! // Write the product and what it refers to, renumbered from #1.
//! let mut out = Vec::new();
//! Writer::new(exchange)
//!     .numbering(Numbering::Dense)
//!     .write_selection([0, 1, 2], &mut out)?;
//! # Ok::<(), stepq::Error>(())
//! ```
//!
//! ## Status
//!
//! Pre-1.0. The public API will change. See `ROADMAP.md` in the
//! repository for what is planned and `docs/ARCHITECTURE.md` for the
//! design decisions this crate is built on.

#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod assemble;
pub mod diff;
pub mod error;
pub mod express;
pub mod info;
pub mod lint;
pub mod model;
pub mod p21;
pub mod pmi;
pub mod props;
pub mod strip;

pub use error::{Error, Result};
