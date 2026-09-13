//! Product structure: product definitions and the assembly usages that
//! nest them.
//!
//! Built from the entity graph alone, following `docs/ARCHITECTURE.md`,
//! "Assembly structure". See [`ProductStructure`] for the rules.

use std::borrow::Cow;
use std::collections::HashMap;

use super::Graph;
use crate::p21::{Exchange, Instance, Param, Record};

const NAUO: &str = "NEXT_ASSEMBLY_USAGE_OCCURRENCE";
const QACU: &str = "QUANTIFIED_ASSEMBLY_COMPONENT_USAGE";

/// Assembly trees deeper than this are not descended further, so crafted
/// input cannot exhaust the stack.
const MAX_DEPTH: usize = 1024;

/// The product structure of a file: its product definitions and the
/// assembly usages that nest them.
///
/// * A product definition is a `PRODUCT_DEFINITION`, or whatever an
///   assembly usage names as a parent or child.
/// * An assembly usage is a `NEXT_ASSEMBLY_USAGE_OCCURRENCE` or a
///   `QUANTIFIED_ASSEMBLY_COMPONENT_USAGE`, simple or complex. Its relating
///   and related definitions decide parent and child — never the direction
///   of a shape relationship, which real files reverse.
/// * A placement is found in one of two ways, and identified, never
///   evaluated:
///   - a `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION` ties the usage to a
///     representation relationship and its transformation;
///   - otherwise, a `MAPPED_ITEM` in the assembly's shape representation
///     maps the component's shape through a `REPRESENTATION_MAP`. Mapped
///     items say which component they place but not which usage, so they
///     are matched to that component's usages in file order, and only when
///     there are exactly as many mapped items as usages.
///
/// See `docs/ARCHITECTURE.md`, "Assembly structure".
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProductStructure {
    definitions: Vec<Definition>,
    usages: Vec<Usage>,
    roots: Vec<usize>,
    #[cfg_attr(feature = "serde", serde(skip))]
    children: Vec<Vec<usize>>,
}

/// A product definition: one view (usually the design view) of a product
/// version.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Definition {
    /// The `#id` of the definition instance.
    pub instance: u64,
    /// Its upper-cased entity type, such as `PRODUCT_DEFINITION`.
    pub entity: String,
    /// `PRODUCT_DEFINITION.id`.
    pub id: Option<String>,
    /// `PRODUCT_DEFINITION.description`.
    pub description: Option<String>,
    /// `PRODUCT_DEFINITION_FORMATION.id`, usually a version.
    pub version: Option<String>,
    /// The product defined.
    pub product: Option<Product>,
    /// `#id`s of the shape representations attached through
    /// `PRODUCT_DEFINITION_SHAPE` and `SHAPE_DEFINITION_REPRESENTATION`.
    pub shape_representations: Vec<u64>,
}

/// A `PRODUCT`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Product {
    /// The `#id` of the `PRODUCT` instance.
    pub instance: u64,
    /// `PRODUCT.id`, often a part number.
    pub id: Option<String>,
    /// `PRODUCT.name`.
    pub name: Option<String>,
    /// `PRODUCT.description`.
    pub description: Option<String>,
}

/// The kind of an assembly usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum UsageKind {
    /// `NEXT_ASSEMBLY_USAGE_OCCURRENCE`: one placed occurrence.
    NextAssemblyUsageOccurrence,
    /// `QUANTIFIED_ASSEMBLY_COMPONENT_USAGE`, possibly combined with the
    /// above: a usage with an explicit quantity.
    QuantifiedAssemblyComponentUsage,
}

/// One use of a component definition inside an assembly definition.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Usage {
    /// The `#id` of the usage instance.
    pub instance: u64,
    /// Which kind of usage.
    pub kind: UsageKind,
    /// The usage's `id` attribute.
    pub id: Option<String>,
    /// The usage's `name` attribute, often an instance name.
    pub name: Option<String>,
    /// The usage's `description` attribute.
    pub description: Option<String>,
    /// `reference_designator`, such as `R12`.
    pub reference_designator: Option<String>,
    /// The assembly: an index into [`ProductStructure::definitions`].
    pub parent: usize,
    /// The component: an index into [`ProductStructure::definitions`].
    pub child: usize,
    /// The quantity of a quantified usage; `None` means one occurrence.
    pub quantity: Option<f64>,
    /// The entities that place the component, if any.
    pub placement: Option<Placement>,
}

/// The entities that place a component in its assembly. Identified only;
/// no transformation is evaluated.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[non_exhaustive]
pub enum Placement {
    /// A `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION` ties the usage to a
    /// representation relationship between the component's and the
    /// assembly's shapes.
    ShapeRelationship {
        /// The `#id` of the `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION`.
        context_dependent_shape_representation: u64,
        /// The `#id` of the representation relationship it names, often a
        /// complex instance with
        /// `REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION`.
        relationship: u64,
        /// The `#id` of the transformation, typically an
        /// `ITEM_DEFINED_TRANSFORMATION`.
        transformation: Option<u64>,
        /// `Some(true)` if the relationship names the assembly's shape as
        /// `rep_1` and the component's as `rep_2`, the reverse of the usual
        /// order; `None` if neither side could be matched to a shape.
        reversed: Option<bool>,
    },
    /// A `MAPPED_ITEM` in the assembly's shape representation maps the
    /// component's shape through a `REPRESENTATION_MAP`.
    MappedItem {
        /// The `#id` of the `MAPPED_ITEM`.
        mapped_item: u64,
        /// The `#id` of its `REPRESENTATION_MAP`.
        representation_map: u64,
        /// The `#id` of the map's `mapping_origin`, a placement in the
        /// component's shape.
        origin: Option<u64>,
        /// The `#id` of the item's `mapping_target`, where that placement
        /// goes in the assembly's shape.
        target: Option<u64>,
    },
}

impl Placement {
    /// For a shape relationship, whether `rep_1` and `rep_2` are reversed;
    /// `None` for a mapped item, which has no direction to get wrong.
    pub fn reversed(&self) -> Option<bool> {
        match self {
            Self::ShapeRelationship { reversed, .. } => *reversed,
            Self::MappedItem { .. } => None,
        }
    }
}

/// One line of a bill of materials.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct BomLine {
    /// An index into [`ProductStructure::definitions`].
    pub definition: usize,
    /// Total quantity under the root.
    pub quantity: f64,
    /// True if the definition has components of its own.
    pub is_assembly: bool,
}

/// One line of a multi-level (indented) bill of materials; see
/// [`ProductStructure::bom_tree`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct BomNode {
    /// An index into [`ProductStructure::definitions`].
    pub definition: usize,
    /// Quantity per parent: this component's usages in its parent, added
    /// up. 1 for the root.
    pub quantity: f64,
    /// Total quantity in the root: the quantities along the path multiplied.
    pub total: f64,
    /// True if the definition has components of its own, even when they are
    /// not listed under this node.
    pub is_assembly: bool,
    /// True if this sub-assembly was already expanded earlier in the tree,
    /// so its components are not repeated here.
    pub repeated: bool,
    /// True if components exist but were cut off: by the depth limit, by a
    /// cycle, or by the size limit.
    pub truncated: bool,
    /// The components, one node per distinct component, in order of first
    /// use.
    pub components: Vec<BomNode>,
}

/// Options for [`ProductStructure::bom_tree`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct BomTreeOptions {
    /// List components at most this many levels below the root; `None`
    /// lists all. A depth of 1 lists the root's direct components.
    pub depth: Option<usize>,
    /// Expand each sub-assembly only the first time it appears; later
    /// occurrences are marked [`repeated`](BomNode::repeated).
    pub dedupe: bool,
}

impl Default for BomTreeOptions {
    fn default() -> Self {
        Self {
            depth: None,
            dedupe: true,
        }
    }
}

impl BomTreeOptions {
    /// Options with the given depth limit and deduplication.
    pub fn new(depth: Option<usize>, dedupe: bool) -> Self {
        Self { depth, dedupe }
    }
}

/// Multi-level bills of materials larger than this are truncated, so that
/// expanding every repeated sub-assembly of a crafted file cannot exhaust
/// memory.
const MAX_BOM_NODES: usize = 1_000_000;

impl ProductStructure {
    /// Reads the product structure of the file behind `graph`. Usages whose
    /// parent or child is undefined are skipped.
    pub fn new(graph: &Graph<'_>) -> Self {
        let exchange = graph.exchange();
        let mut builder = Builder {
            graph,
            exchange,
            definitions: Vec::new(),
            by_node: HashMap::new(),
        };
        for node in exchange.instances_of("PRODUCT_DEFINITION") {
            builder.definition(node);
        }

        let mut usage_nodes: Vec<usize> = exchange
            .instances_of(NAUO)
            .chain(exchange.instances_of(QACU))
            .collect();
        usage_nodes.sort_unstable();
        usage_nodes.dedup();

        let mut usages = Vec::new();
        let mut placed_by = Vec::new();
        for node in usage_nodes {
            let instance = graph.instance(node);
            let Some(fields) = usage_fields(exchange, instance) else {
                continue;
            };
            let (Some(parent), Some(child)) =
                (graph.node(fields.relating), graph.node(fields.related))
            else {
                continue;
            };
            let parent = builder.definition(parent);
            let child = builder.definition(child);
            usages.push(Usage {
                instance: instance.id,
                kind: fields.kind,
                id: fields.id,
                name: fields.name,
                description: fields.description,
                reference_designator: fields.reference_designator,
                parent,
                child,
                quantity: fields.quantity.and_then(|id| measure_value(graph, id)),
                placement: None,
            });
            placed_by.push(node);
        }

        let definitions = builder.definitions;
        for (usage, &node) in usages.iter_mut().zip(&placed_by) {
            usage.placement = shape_relationship(
                graph,
                node,
                &definitions[usage.parent].shape_representations,
                &definitions[usage.child].shape_representations,
            );
        }

        // Usages still unplaced may be placed by mapped items, which name a
        // component but not a usage: match them per (assembly, component).
        let mut unplaced: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
        for (index, usage) in usages.iter().enumerate() {
            if usage.placement.is_none() {
                unplaced
                    .entry((usage.parent, usage.child))
                    .or_default()
                    .push(index);
            }
        }
        for ((parent, child), indices) in unplaced {
            let items = mapped_items(
                graph,
                &definitions[parent].shape_representations,
                &definitions[child].shape_representations,
            );
            if items.len() == indices.len() {
                for (index, placement) in indices.into_iter().zip(items) {
                    usages[index].placement = Some(placement);
                }
            }
        }

        let mut children = vec![Vec::new(); definitions.len()];
        let mut is_child = vec![false; definitions.len()];
        for (index, usage) in usages.iter().enumerate() {
            children[usage.parent].push(index);
            is_child[usage.child] = true;
        }
        let roots = (0..definitions.len()).filter(|&d| !is_child[d]).collect();

        Self {
            definitions,
            usages,
            roots,
            children,
        }
    }

    /// Every product definition, in file order of first appearance.
    pub fn definitions(&self) -> &[Definition] {
        &self.definitions
    }

    /// Every assembly usage, in file order.
    pub fn usages(&self) -> &[Usage] {
        &self.usages
    }

    /// Definitions that are not a component of anything: top-level
    /// assemblies and stand-alone parts.
    pub fn roots(&self) -> &[usize] {
        &self.roots
    }

    /// The usages whose parent is `definition`, in file order.
    ///
    /// # Panics
    ///
    /// Panics if `definition` is out of range.
    pub fn children(&self, definition: usize) -> impl ExactSizeIterator<Item = &Usage> {
        self.children[definition]
            .iter()
            .map(|&index| &self.usages[index])
    }

    /// The components of `definition`, each with every usage of it, in order
    /// of first use.
    ///
    /// # Panics
    ///
    /// Panics if `definition` is out of range.
    pub fn components(&self, definition: usize) -> Vec<(usize, Vec<&Usage>)> {
        let mut components: Vec<(usize, Vec<&Usage>)> = Vec::new();
        for usage in self.children(definition) {
            match components
                .iter_mut()
                .find(|(child, _)| *child == usage.child)
            {
                Some((_, usages)) => usages.push(usage),
                None => components.push((usage.child, vec![usage])),
            }
        }
        components
    }

    /// Every distinct definition below `root` with its total quantity, in
    /// order of first appearance, depth first. Quantities multiply down the
    /// tree and add up across branches; a usage without a quantity counts
    /// once. A usage that would re-enter its own ancestry is ignored.
    ///
    /// # Panics
    ///
    /// Panics if `root` is out of range.
    pub fn bill_of_materials(&self, root: usize) -> Vec<BomLine> {
        let mut memo = vec![None; self.definitions.len()];
        let mut visiting = vec![false; self.definitions.len()];
        self.totals(root, &mut memo, &mut visiting, 0)
            .into_iter()
            .map(|(definition, quantity)| BomLine {
                definition,
                quantity,
                is_assembly: !self.children[definition].is_empty(),
            })
            .collect()
    }

    /// The multi-level bill of materials of `root`: the root with its
    /// components, each with its own components, one node per distinct
    /// component of each assembly. Quantities per parent add up the usages
    /// of a component in that parent; totals multiply them along the path.
    ///
    /// # Panics
    ///
    /// Panics if `root` is out of range.
    pub fn bom_tree(&self, root: usize, options: BomTreeOptions) -> BomNode {
        let mut state = BomWalk {
            options,
            expanded: vec![false; self.definitions.len()],
            path: vec![false; self.definitions.len()],
            budget: MAX_BOM_NODES,
        };
        self.bom_node(root, 1.0, 1.0, 0, &mut state)
    }

    fn bom_node(
        &self,
        definition: usize,
        quantity: f64,
        total: f64,
        level: usize,
        state: &mut BomWalk,
    ) -> BomNode {
        state.budget = state.budget.saturating_sub(1);
        let mut node = BomNode {
            definition,
            quantity,
            total,
            is_assembly: !self.children[definition].is_empty(),
            repeated: false,
            truncated: false,
            components: Vec::new(),
        };
        if !node.is_assembly {
            return node;
        }
        if state.options.dedupe && state.expanded[definition] {
            node.repeated = true;
            return node;
        }
        let too_deep = state.options.depth.is_some_and(|depth| level >= depth);
        if too_deep || state.path[definition] || level >= MAX_DEPTH || state.budget == 0 {
            node.truncated = true;
            return node;
        }
        state.expanded[definition] = true;
        state.path[definition] = true;
        for (child, usages) in self.components(definition) {
            if state.budget == 0 {
                node.truncated = true;
                break;
            }
            let each: f64 = usages.iter().map(|u| u.quantity.unwrap_or(1.0)).sum();
            let component = self.bom_node(child, each, total * each, level + 1, state);
            node.components.push(component);
        }
        state.path[definition] = false;
        node
    }

    fn totals(
        &self,
        definition: usize,
        memo: &mut [Option<Vec<(usize, f64)>>],
        visiting: &mut [bool],
        depth: usize,
    ) -> Vec<(usize, f64)> {
        if let Some(totals) = &memo[definition] {
            return totals.clone();
        }
        let mut totals: Vec<(usize, f64)> = Vec::new();
        let mut positions: HashMap<usize, usize> = HashMap::new();
        let mut add = |definition: usize, quantity: f64| {
            if let Some(&at) = positions.get(&definition) {
                totals[at].1 += quantity;
            } else {
                positions.insert(definition, totals.len());
                totals.push((definition, quantity));
            }
        };
        if depth < MAX_DEPTH {
            visiting[definition] = true;
            for &index in &self.children[definition] {
                let usage = &self.usages[index];
                if visiting[usage.child] {
                    continue;
                }
                let quantity = usage.quantity.unwrap_or(1.0);
                add(usage.child, quantity);
                for (descendant, each) in self.totals(usage.child, memo, visiting, depth + 1) {
                    add(descendant, quantity * each);
                }
            }
            visiting[definition] = false;
        }
        memo[definition] = Some(totals.clone());
        totals
    }
}

struct BomWalk {
    options: BomTreeOptions,
    /// Sub-assemblies already expanded somewhere in the tree.
    expanded: Vec<bool>,
    /// Sub-assemblies on the path from the root to the current node.
    path: Vec<bool>,
    /// Nodes that may still be created.
    budget: usize,
}

struct Builder<'g, 'a> {
    graph: &'g Graph<'a>,
    exchange: &'g Exchange<'a>,
    definitions: Vec<Definition>,
    by_node: HashMap<usize, usize>,
}

impl Builder<'_, '_> {
    /// The index of the definition at graph `node`, reading it on first use.
    fn definition(&mut self, node: usize) -> usize {
        if let Some(&index) = self.by_node.get(&node) {
            return index;
        }
        let graph = self.graph;
        let exchange = self.exchange;
        let instance = graph.instance(node);
        let record = primary_record(exchange, instance, "PRODUCT_DEFINITION");
        let entity = record.map(|r| upper(r.name())).unwrap_or_default();
        let standard =
            entity == "PRODUCT_DEFINITION" || entity.starts_with("PRODUCT_DEFINITION_WITH_");

        let mut definition = Definition {
            instance: instance.id,
            entity,
            id: None,
            description: None,
            version: None,
            product: None,
            shape_representations: shape_representations(graph, node),
        };
        if let (true, Some(record)) = (standard, record) {
            definition.id = text(record.param(0).as_ref());
            definition.description = text(record.param(1).as_ref());
            let formation = reference(record.param(2).as_ref())
                .and_then(|id| graph.node(id))
                .and_then(|node| exchange.records(graph.instance(node)).next());
            if let Some(formation) = formation {
                definition.version = text(formation.param(0).as_ref());
                definition.product = reference(formation.param(2).as_ref())
                    .and_then(|id| graph.node(id))
                    .and_then(|node| {
                        let product = graph.instance(node);
                        let record = primary_record(exchange, product, "PRODUCT")?;
                        Some(Product {
                            instance: product.id,
                            id: text(record.param(0).as_ref()),
                            name: text(record.param(1).as_ref()),
                            description: text(record.param(2).as_ref()),
                        })
                    });
            }
        }

        let index = self.definitions.len();
        self.definitions.push(definition);
        self.by_node.insert(node, index);
        index
    }
}

struct UsageFields {
    kind: UsageKind,
    id: Option<String>,
    name: Option<String>,
    description: Option<String>,
    relating: u64,
    related: u64,
    reference_designator: Option<String>,
    quantity: Option<u64>,
}

/// Reads a usage, simple or complex. In a complex instance each attribute
/// lives in the partial record of the entity that declares it.
fn usage_fields(exchange: &Exchange<'_>, instance: &Instance) -> Option<UsageFields> {
    let records: Vec<Record<'_>> = exchange.records(instance).collect();
    let find = |name: &str| records.iter().copied().find(|r| r.is(name));
    let quantified = find(QACU).is_some();
    let kind = if quantified {
        UsageKind::QuantifiedAssemblyComponentUsage
    } else {
        UsageKind::NextAssemblyUsageOccurrence
    };

    let (relationship, designator, quantity) = if instance.is_complex() {
        (
            find("PRODUCT_DEFINITION_RELATIONSHIP")?,
            find("ASSEMBLY_COMPONENT_USAGE").and_then(|r| r.param(0)),
            find(QACU).and_then(|r| r.param(0)),
        )
    } else {
        let record = *records.first()?;
        (
            record,
            record.param(5),
            if quantified { record.param(6) } else { None },
        )
    };
    Some(UsageFields {
        kind,
        id: text(relationship.param(0).as_ref()),
        name: text(relationship.param(1).as_ref()),
        description: text(relationship.param(2).as_ref()),
        relating: reference(relationship.param(3).as_ref())?,
        related: reference(relationship.param(4).as_ref())?,
        reference_designator: text(designator.as_ref()),
        quantity: reference(quantity.as_ref()),
    })
}

/// The shape representations attached to a definition:
/// `SHAPE_DEFINITION_REPRESENTATION(PRODUCT_DEFINITION_SHAPE(.., definition), rep)`.
fn shape_representations(graph: &Graph<'_>, definition: usize) -> Vec<u64> {
    let id = graph.instance(definition).id;
    let mut representations = Vec::new();
    for &shape in graph.referenced_by(definition) {
        if !record_references(graph, shape, "PRODUCT_DEFINITION_SHAPE", 2, id) {
            continue;
        }
        let shape_id = graph.instance(shape).id;
        for &link in graph.referenced_by(shape) {
            if record_references(graph, link, "SHAPE_DEFINITION_REPRESENTATION", 0, shape_id) {
                if let Some(representation) = first_reference(graph, link, 1) {
                    representations.push(representation);
                }
            }
        }
    }
    representations
}

/// `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(relationship, PRODUCT_DEFINITION_SHAPE(.., usage))`.
fn shape_relationship(
    graph: &Graph<'_>,
    usage: usize,
    parent_shapes: &[u64],
    child_shapes: &[u64],
) -> Option<Placement> {
    let exchange = graph.exchange();
    let usage_id = graph.instance(usage).id;
    for &shape in graph.referenced_by(usage) {
        if !record_references(graph, shape, "PRODUCT_DEFINITION_SHAPE", 2, usage_id) {
            continue;
        }
        let shape_id = graph.instance(shape).id;
        for &link in graph.referenced_by(shape) {
            if !record_references(
                graph,
                link,
                "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION",
                1,
                shape_id,
            ) {
                continue;
            }
            let Some(relationship) = first_reference(graph, link, 0) else {
                continue;
            };
            let Some(node) = graph.node(relationship) else {
                continue;
            };
            let instance = graph.instance(node);
            let records: Vec<Record<'_>> = exchange.records(instance).collect();
            let (base, transformation) = if instance.is_complex() {
                (
                    records
                        .iter()
                        .copied()
                        .find(|r| r.is("REPRESENTATION_RELATIONSHIP")),
                    records
                        .iter()
                        .find(|r| r.is("REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION"))
                        .and_then(|r| reference(r.param(0).as_ref())),
                )
            } else {
                let record = records.first().copied();
                let transformation = record
                    .filter(|r| upper(r.name()).ends_with("WITH_TRANSFORMATION"))
                    .and_then(|r| reference(r.param(4).as_ref()));
                (record, transformation)
            };
            let rep_1 = base.and_then(|r| reference(r.param(2).as_ref()));
            let rep_2 = base.and_then(|r| reference(r.param(3).as_ref()));
            let is_child = |rep: Option<u64>| rep.is_some_and(|rep| child_shapes.contains(&rep));
            let is_parent = |rep: Option<u64>| rep.is_some_and(|rep| parent_shapes.contains(&rep));
            let reversed = if is_child(rep_1) || is_parent(rep_2) {
                Some(false)
            } else if is_child(rep_2) || is_parent(rep_1) {
                Some(true)
            } else {
                None
            };
            return Some(Placement::ShapeRelationship {
                context_dependent_shape_representation: graph.instance(link).id,
                relationship,
                transformation,
                reversed,
            });
        }
    }
    None
}

/// The `MAPPED_ITEM`s among the items of the assembly's shape
/// representations whose `REPRESENTATION_MAP` maps one of the component's
/// shape representations, in file order.
fn mapped_items(graph: &Graph<'_>, parent_shapes: &[u64], child_shapes: &[u64]) -> Vec<Placement> {
    let exchange = graph.exchange();
    let mut placements = Vec::new();
    for &shape in parent_shapes {
        let items = graph
            .node(shape)
            .and_then(|node| exchange.records(graph.instance(node)).next())
            .and_then(|record| record.param(1))
            .and_then(|items| items.list());
        for id in items
            .into_iter()
            .flatten()
            .filter_map(|item| item.reference())
        {
            let Some(node) = graph.node(id) else {
                continue;
            };
            let Some(item) = primary_record(exchange, graph.instance(node), "MAPPED_ITEM")
                .filter(|r| r.is("MAPPED_ITEM"))
            else {
                continue;
            };
            let Some(map_id) = reference(item.param(1).as_ref()) else {
                continue;
            };
            let Some(map) = graph
                .node(map_id)
                .and_then(|node| exchange.records(graph.instance(node)).next())
                .filter(|r| r.is("REPRESENTATION_MAP"))
            else {
                continue;
            };
            if reference(map.param(1).as_ref()).is_some_and(|rep| child_shapes.contains(&rep)) {
                placements.push(Placement::MappedItem {
                    mapped_item: id,
                    representation_map: map_id,
                    origin: reference(map.param(0).as_ref()),
                    target: reference(item.param(2).as_ref()),
                });
            }
        }
    }
    placements
}

/// The numeric value of a `MEASURE_WITH_UNIT`, whose first attribute is a
/// typed value such as `COUNT_MEASURE(4.)`.
fn measure_value(graph: &Graph<'_>, id: u64) -> Option<f64> {
    let node = graph.node(id)?;
    let exchange = graph.exchange();
    let record = primary_record(exchange, graph.instance(node), "MEASURE_WITH_UNIT")?;
    let value = record.param(0)?;
    let literal = match value.typed() {
        Some(typed) => typed.param(0)?.literal()?,
        None => value.literal()?,
    };
    literal.to_f64()
}

/// True if `node`'s first record is `entity` and its attribute `index`
/// refers to `target`.
fn record_references(
    graph: &Graph<'_>,
    node: usize,
    entity: &str,
    index: usize,
    target: u64,
) -> bool {
    graph
        .exchange()
        .records(graph.instance(node))
        .next()
        .is_some_and(|r| r.is(entity) && reference(r.param(index).as_ref()) == Some(target))
}

fn first_reference(graph: &Graph<'_>, node: usize, index: usize) -> Option<u64> {
    let record = graph.exchange().records(graph.instance(node)).next()?;
    reference(record.param(index).as_ref())
}

/// The partial record named `name`, or the first record.
fn primary_record<'e>(
    exchange: &'e Exchange<'_>,
    instance: &Instance,
    name: &str,
) -> Option<Record<'e>> {
    let mut records = exchange.records(instance);
    records
        .clone()
        .find(|r| r.is(name))
        .or_else(|| records.next())
}

fn reference(param: Option<&Param<'_>>) -> Option<u64> {
    param.and_then(Param::reference)
}

/// A string attribute's decoded value; `None` if absent, not a string, or
/// blank.
fn text(param: Option<&Param<'_>>) -> Option<String> {
    let Some(Param::String(literal)) = param else {
        return None;
    };
    let value = literal.decode().map_or_else(
        |_| String::from_utf8_lossy(literal.text()).into_owned(),
        Cow::into_owned,
    );
    (!value.trim().is_empty()).then_some(value)
}

fn upper(name: &[u8]) -> String {
    String::from_utf8_lossy(name).to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::parse;

    /// Assembly A: two of sub-assembly B (each with three C) and a
    /// quantified usage of four D. B's first usage is placed normally; C's
    /// first usage names the parent shape as `rep_1`.
    const SAMPLE: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('AP242'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('design');
#2=PRODUCT_CONTEXT('',#1,'mechanical');
#3=PRODUCT_DEFINITION_CONTEXT('part definition',#1,'design');
#11=PRODUCT('A','assembly','',(#2));
#12=PRODUCT_DEFINITION_FORMATION('1','',#11);
#10=PRODUCT_DEFINITION('a-def','',#12,#3);
#21=PRODUCT('B','bracket assembly','',(#2));
#22=PRODUCT_DEFINITION_FORMATION_WITH_SPECIFIED_SOURCE('2','',#21,.MADE.);
#20=PRODUCT_DEFINITION('b-def','',#22,#3);
#31=PRODUCT('C','bolt','',(#2));
#32=PRODUCT_DEFINITION_FORMATION('1','',#31);
#30=PRODUCT_DEFINITION('c-def','',#32,#3);
#41=PRODUCT('D','plate','',(#2));
#42=PRODUCT_DEFINITION_FORMATION('1','',#41);
#40=PRODUCT_DEFINITION('d-def','',#42,#3);
#50=SHAPE_REPRESENTATION('a',(#60),#1);
#51=SHAPE_REPRESENTATION('b',(#60),#1);
#52=SHAPE_REPRESENTATION('c',(#60),#1);
#60=AXIS2_PLACEMENT_3D('',#61,$,$);
#61=CARTESIAN_POINT('',(0.,0.,0.));
#70=PRODUCT_DEFINITION_SHAPE('','',#10);
#71=PRODUCT_DEFINITION_SHAPE('','',#20);
#72=PRODUCT_DEFINITION_SHAPE('','',#30);
#80=SHAPE_DEFINITION_REPRESENTATION(#70,#50);
#81=SHAPE_DEFINITION_REPRESENTATION(#71,#51);
#82=SHAPE_DEFINITION_REPRESENTATION(#72,#52);
#100=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u1','B1','',#10,#20,$);
#101=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u2','B2','',#10,#20,$);
#102=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u3','C1','',#20,#30,'X1');
#103=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u4','C2','',#20,#30,'X2');
#104=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u5','C3','',#20,#30,'X3');
#105=(ASSEMBLY_COMPONENT_USAGE($)NEXT_ASSEMBLY_USAGE_OCCURRENCE()PRODUCT_DEFINITION_RELATIONSHIP('u6','D','',#10,#40)PRODUCT_DEFINITION_USAGE()QUANTIFIED_ASSEMBLY_COMPONENT_USAGE(#106));
#106=MEASURE_WITH_UNIT(COUNT_MEASURE(4.),#1);
#90=PRODUCT_DEFINITION_SHAPE('','',#100);
#91=(REPRESENTATION_RELATIONSHIP('','',#51,#50)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#92)SHAPE_REPRESENTATION_RELATIONSHIP());
#92=ITEM_DEFINED_TRANSFORMATION('','',#60,#60);
#93=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#91,#90);
#94=PRODUCT_DEFINITION_SHAPE('','',#102);
#95=REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION('','',#51,#52,#92);
#96=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#95,#94);
ENDSEC;END-ISO-10303-21;";

    /// Assembly A places two of part C only through mapped items, the way
    /// some exporters (such as `STEPnet`) do.
    fn mapped_sample(items: &str) -> String {
        format!(
            "ISO-10303-21;HEADER;FILE_SCHEMA(('X'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('design');
#10=PRODUCT_DEFINITION('a','',$,$);
#30=PRODUCT_DEFINITION('c','',$,$);
#50=ADVANCED_BREP_SHAPE_REPRESENTATION('a',({items}),#1);
#52=ADVANCED_BREP_SHAPE_REPRESENTATION('c',(#60),#1);
#60=AXIS2_PLACEMENT_3D('',#61,$,$);
#61=CARTESIAN_POINT('',(0.,0.,0.));
#62=AXIS2_PLACEMENT_3D('',#61,$,$);
#70=PRODUCT_DEFINITION_SHAPE('','',#10);
#72=PRODUCT_DEFINITION_SHAPE('','',#30);
#80=SHAPE_DEFINITION_REPRESENTATION(#70,#50);
#82=SHAPE_DEFINITION_REPRESENTATION(#72,#52);
#100=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','','',#10,#30,$);
#101=NEXT_ASSEMBLY_USAGE_OCCURRENCE('2','','',#10,#30,$);
#150=REPRESENTATION_MAP(#60,#52);
#200=MAPPED_ITEM('',#150,#60);
#201=MAPPED_ITEM('',#150,#62);
ENDSEC;END-ISO-10303-21;"
        )
    }

    fn structure_of(src: &str) -> ProductStructure {
        ProductStructure::new(&Graph::new(parse(src.as_bytes()).unwrap()).unwrap())
    }

    fn structure() -> ProductStructure {
        structure_of(SAMPLE)
    }

    #[test]
    fn definitions_and_products() {
        let structure = structure();
        let instances: Vec<u64> = structure.definitions().iter().map(|d| d.instance).collect();
        assert_eq!(instances, [10, 20, 30, 40]);
        let b = &structure.definitions()[1];
        assert_eq!(b.id.as_deref(), Some("b-def"));
        assert_eq!(b.version.as_deref(), Some("2"));
        let product = b.product.as_ref().unwrap();
        assert_eq!(
            (
                product.instance,
                product.id.as_deref(),
                product.name.as_deref()
            ),
            (21, Some("B"), Some("bracket assembly"))
        );
        assert_eq!(product.description, None, "blank strings are omitted");
        assert_eq!(structure.definitions()[0].shape_representations, [50]);
        assert!(structure.definitions()[3].shape_representations.is_empty());
        assert_eq!(structure.roots(), [0]);
    }

    #[test]
    fn usages_simple_and_complex() {
        let structure = structure();
        let usages = structure.usages();
        assert_eq!(usages.len(), 6);
        assert_eq!(
            usages
                .iter()
                .map(|u| (u.instance, u.parent, u.child))
                .collect::<Vec<_>>(),
            [
                (100, 0, 1),
                (101, 0, 1),
                (102, 1, 2),
                (103, 1, 2),
                (104, 1, 2),
                (105, 0, 3)
            ]
        );
        assert_eq!(usages[2].reference_designator.as_deref(), Some("X1"));
        assert_eq!(usages[0].kind, UsageKind::NextAssemblyUsageOccurrence);

        let quantified = &usages[5];
        assert_eq!(quantified.kind, UsageKind::QuantifiedAssemblyComponentUsage);
        assert_eq!(quantified.id.as_deref(), Some("u6"));
        assert_eq!(
            quantified.quantity.map(|q| q.to_string()).as_deref(),
            Some("4")
        );
        assert_eq!(quantified.reference_designator, None);
    }

    #[test]
    fn shape_relationship_placements_and_reversal() {
        let structure = structure();
        let usages = structure.usages();
        assert_eq!(
            usages[0].placement,
            Some(Placement::ShapeRelationship {
                context_dependent_shape_representation: 93,
                relationship: 91,
                transformation: Some(92),
                reversed: Some(false),
            })
        );
        assert_eq!(
            usages[2].placement,
            Some(Placement::ShapeRelationship {
                context_dependent_shape_representation: 96,
                relationship: 95,
                transformation: Some(92),
                reversed: Some(true),
            })
        );
        assert_eq!(usages[2].placement.as_ref().unwrap().reversed(), Some(true));
        assert_eq!(usages[1].placement, None);
    }

    #[test]
    fn mapped_items_place_usages_in_order() {
        let structure = structure_of(&mapped_sample("#200,#61,#201"));
        let placements: Vec<Option<Placement>> = structure
            .usages()
            .iter()
            .map(|u| u.placement.clone())
            .collect();
        assert_eq!(
            placements,
            [
                Some(Placement::MappedItem {
                    mapped_item: 200,
                    representation_map: 150,
                    origin: Some(60),
                    target: Some(60),
                }),
                Some(Placement::MappedItem {
                    mapped_item: 201,
                    representation_map: 150,
                    origin: Some(60),
                    target: Some(62),
                }),
            ]
        );
        assert_eq!(placements[0].as_ref().unwrap().reversed(), None);
    }

    #[test]
    fn mapped_items_are_not_guessed_when_counts_differ() {
        let structure = structure_of(&mapped_sample("#200"));
        assert!(structure.usages().iter().all(|u| u.placement.is_none()));
    }

    #[test]
    fn components_group_repeated_usages() {
        let structure = structure();
        let grouped: Vec<(usize, Vec<u64>)> = structure
            .components(0)
            .into_iter()
            .map(|(child, usages)| (child, usages.iter().map(|u| u.instance).collect()))
            .collect();
        assert_eq!(grouped, [(1, vec![100, 101]), (3, vec![105])]);
        assert_eq!(structure.children(1).len(), 3);
        assert_eq!(structure.children(2).len(), 0);
    }

    #[test]
    fn bill_of_materials_multiplies_down_and_adds_across() {
        let structure = structure();
        let lines: Vec<(usize, String, bool)> = structure
            .bill_of_materials(0)
            .iter()
            .map(|l| (l.definition, l.quantity.to_string(), l.is_assembly))
            .collect();
        assert_eq!(
            lines,
            [
                (1, "2".to_owned(), true),
                (2, "6".to_owned(), false),
                (3, "4".to_owned(), false)
            ]
        );
        assert!(structure.bill_of_materials(3).is_empty());
    }

    #[test]
    fn cycles_do_not_loop() {
        let src = "ISO-10303-21;HEADER;FILE_SCHEMA(('X'));ENDSEC;DATA;
#1=PRODUCT_DEFINITION('a','',$,$);
#2=PRODUCT_DEFINITION('b','',$,$);
#3=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','','',#1,#2,$);
#4=NEXT_ASSEMBLY_USAGE_OCCURRENCE('2','','',#2,#1,$);
#5=NEXT_ASSEMBLY_USAGE_OCCURRENCE('3','','',#9,#1,$);
ENDSEC;END-ISO-10303-21;";
        let structure = ProductStructure::new(&Graph::build(parse(src.as_bytes()).unwrap()));
        assert_eq!(
            structure.usages().len(),
            2,
            "a usage with an undefined parent is skipped"
        );
        assert!(
            structure.roots().is_empty(),
            "every definition is someone's child"
        );
        let lines = structure.bill_of_materials(0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].definition, 1);

        let tree = structure.bom_tree(0, BomTreeOptions::default());
        assert_eq!(
            flatten(&tree),
            [
                (0, 0, "1".into(), "1".into(), ""),
                (1, 1, "1".into(), "1".into(), ""),
                (2, 0, "1".into(), "1".into(), "repeated")
            ]
        );
        let tree = structure.bom_tree(0, BomTreeOptions::new(None, false));
        assert_eq!(flatten(&tree)[2].4, "truncated", "cycles are cut off");
    }

    /// (level, definition, quantity, total, mark) for every node, depth first.
    fn flatten(node: &BomNode) -> Vec<(usize, usize, String, String, &'static str)> {
        fn walk(
            node: &BomNode,
            level: usize,
            out: &mut Vec<(usize, usize, String, String, &'static str)>,
        ) {
            let mark = if node.repeated {
                "repeated"
            } else if node.truncated {
                "truncated"
            } else {
                ""
            };
            out.push((
                level,
                node.definition,
                node.quantity.to_string(),
                node.total.to_string(),
                mark,
            ));
            for component in &node.components {
                walk(component, level + 1, out);
            }
        }
        let mut out = Vec::new();
        walk(node, 0, &mut out);
        out
    }

    #[test]
    fn bom_tree_groups_components_and_multiplies_totals() {
        let structure = structure();
        let tree = structure.bom_tree(0, BomTreeOptions::default());
        assert_eq!(
            flatten(&tree),
            [
                (0, 0, "1".into(), "1".into(), ""),
                (1, 1, "2".into(), "2".into(), ""),
                (2, 2, "3".into(), "6".into(), ""),
                (1, 3, "4".into(), "4".into(), "")
            ]
        );
        assert!(tree.is_assembly && tree.components[0].is_assembly);
        assert!(!tree.components[1].is_assembly);
    }

    #[test]
    fn bom_tree_depth_limits_the_listing() {
        let structure = structure();
        let tree = structure.bom_tree(0, BomTreeOptions::new(Some(1), true));
        assert_eq!(
            flatten(&tree),
            [
                (0, 0, "1".into(), "1".into(), ""),
                (1, 1, "2".into(), "2".into(), "truncated"),
                (1, 3, "4".into(), "4".into(), "")
            ]
        );
        let root_only = structure.bom_tree(0, BomTreeOptions::new(Some(0), true));
        assert!(root_only.components.is_empty() && root_only.truncated);
    }

    #[test]
    fn bom_tree_expands_repeated_sub_assemblies_once() {
        // A uses B and E; E uses B too; B uses C.
        let src = "ISO-10303-21;HEADER;FILE_SCHEMA(('X'));ENDSEC;DATA;
#1=PRODUCT_DEFINITION('a','',$,$);
#2=PRODUCT_DEFINITION('b','',$,$);
#3=PRODUCT_DEFINITION('e','',$,$);
#4=PRODUCT_DEFINITION('c','',$,$);
#10=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','','',#1,#2,$);
#11=NEXT_ASSEMBLY_USAGE_OCCURRENCE('2','','',#1,#3,$);
#12=NEXT_ASSEMBLY_USAGE_OCCURRENCE('3','','',#3,#2,$);
#13=NEXT_ASSEMBLY_USAGE_OCCURRENCE('4','','',#2,#4,$);
ENDSEC;END-ISO-10303-21;";
        let structure = structure_of(src);
        let marks = |dedupe| {
            flatten(&structure.bom_tree(0, BomTreeOptions::new(None, dedupe)))
                .into_iter()
                .map(|(level, definition, _, _, mark)| (level, definition, mark))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            marks(true),
            [
                (0, 0, ""),
                (1, 1, ""),
                (2, 3, ""),
                (1, 2, ""),
                (2, 1, "repeated")
            ]
        );
        assert_eq!(
            marks(false),
            [
                (0, 0, ""),
                (1, 1, ""),
                (2, 3, ""),
                (1, 2, ""),
                (2, 1, ""),
                (3, 3, "")
            ]
        );
    }
}
