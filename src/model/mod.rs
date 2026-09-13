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
//! checked against the EXPRESS schema at the attribute-count level (see
//! [`express::check`](crate::express::check)). There is no generated
//! struct per entity type.
//!
//! On top of the [`Graph`]: [`ProductStructure`] reads the assembly tree
//! and bill of materials, and [`extract`] computes the self-contained
//! closure of a product definition.

mod assembly;
mod extract;
mod graph;

pub use assembly::{
    BomLine, BomNode, BomTreeOptions, Definition, Placement, Product, ProductStructure, Usage,
    UsageKind,
};
pub use extract::{Extraction, RULES, Rule, RuleKind, extract, orphans};
pub use graph::Graph;
