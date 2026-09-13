//! The in-memory entity graph.
//!
//! A parsed file becomes a set of entity instances keyed by `#id`, plus
//! two indices: forward references (what each instance points at) and
//! back references (what points at each instance).
//!
//! The back index is not an optimisation — it is load-bearing. In STEP,
//! a product's shape, its colours, its PMI and its assembly placement
//! all point *at* the product rather than away from it. A forward
//! closure from a `product_definition` reaches six entities and no
//! geometry.
//!
//! Instances are kept untyped (entity name + attribute tokens) and
//! checked against the EXPRESS schema at the attribute-count level.
//! There is no generated struct per entity type.
//!
//! Implementation status: [`Graph`] builds both reference indices over a
//! parsed [`Exchange`](crate::p21::Exchange); schema checking is not
//! started. See `ROADMAP.md`.

mod assembly;
mod graph;

pub use assembly::{BomLine, Definition, Placement, Product, ProductStructure, Usage, UsageKind};
pub use graph::Graph;
