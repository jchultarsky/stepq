//! Extracting a product definition and everything that belongs to it.
//!
//! A forward closure from a `PRODUCT_DEFINITION` reaches six entities and
//! no geometry: shapes, properties, styles and PMI all point *at* what they
//! describe (`docs/ARCHITECTURE.md`, "The back-reference problem"). Following
//! every back reference is no better — through shared contexts, categories
//! and assembly links it reaches almost the whole file.
//!
//! So an extraction is a fixpoint over a rule table, [`RULES`]: starting
//! from the seeds, take everything an extracted instance refers to, and
//! every referrer that matches a rule through the rule's attribute. Owned
//! referrers are copied whole; shared ones (layer assignments, categories,
//! approvals, …) keep only the list items that are extracted. The table is
//! grown from real files, and whatever no extraction takes can be reported
//! with [`orphans`].

use std::collections::VecDeque;

use super::Graph;
use crate::p21::{Exchange, Instance, Param, Record};

/// How a referrer joins an extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RuleKind {
    /// The referrer belongs to what it refers to — a shape to its product
    /// definition, a style to its item — and is copied whole.
    Owned,
    /// The referrer lists items of many products — a layer, a category, an
    /// approval — and is copied with that list cut down to the extracted
    /// items. It is never followed through the list.
    Shared,
}

/// One entry of the rule table: when an instance of one of `entities`
/// refers to an extracted instance through `attribute`, the instance is
/// extracted too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Rule {
    /// The entity type and those of its subtypes that keep the attribute at
    /// the same position.
    pub entities: &'static [&'static str],
    /// The attribute's name in the EXPRESS schema, lower case.
    pub attribute: &'static str,
    /// The attribute's position in a simple instance's record.
    pub simple: usize,
    /// For complex instances: the partial entity that declares the
    /// attribute and the attribute's position in that partial record, or
    /// `None` if the rule does not apply to complex instances.
    pub partial: Option<(&'static str, usize)>,
    /// Owned or shared.
    pub kind: RuleKind,
}

const fn owned(
    entities: &'static [&'static str],
    attribute: &'static str,
    simple: usize,
    partial: Option<(&'static str, usize)>,
) -> Rule {
    Rule {
        entities,
        attribute,
        simple,
        partial,
        kind: RuleKind::Owned,
    }
}

const fn shared(
    entities: &'static [&'static str],
    attribute: &'static str,
    simple: usize,
    partial: Option<(&'static str, usize)>,
) -> Rule {
    Rule {
        entities,
        attribute,
        simple,
        partial,
        kind: RuleKind::Shared,
    }
}

const SHAPE_ASPECTS: &[&str] = &[
    "SHAPE_ASPECT",
    "ALL_AROUND_SHAPE_ASPECT",
    "APEX",
    "BETWEEN_SHAPE_ASPECT",
    "CENTRE_OF_SYMMETRY",
    "COMMON_DATUM",
    "COMPOSITE_GROUP_SHAPE_ASPECT",
    "COMPOSITE_SHAPE_ASPECT",
    "CONTINUOUS_SHAPE_ASPECT",
    "DATUM",
    "DATUM_FEATURE",
    "DATUM_REFERENCE_COMPARTMENT",
    "DATUM_REFERENCE_ELEMENT",
    "DATUM_SYSTEM",
    "DATUM_TARGET",
    "DERIVED_SHAPE_ASPECT",
    "EXTENSION",
    "GENERAL_DATUM_REFERENCE",
    "GEOMETRIC_ALIGNMENT",
    "PARALLEL_OFFSET",
    "PERPENDICULAR_TO",
    "PLACED_DATUM_TARGET_FEATURE",
    "SYMMETRIC_SHAPE_ASPECT",
    "TANGENT",
    "TOLERANCE_ZONE",
];

const SHAPE_ASPECT_RELATIONSHIPS: &[&str] = &[
    "SHAPE_ASPECT_RELATIONSHIP",
    "ANGULAR_LOCATION",
    "DIMENSIONAL_LOCATION",
    "DIMENSIONAL_LOCATION_WITH_PATH",
    "FEATURE_FOR_DATUM_TARGET_RELATIONSHIP",
    "SHAPE_ASPECT_DERIVING_RELATIONSHIP",
];

const GEOMETRIC_TOLERANCES: &[&str] = &[
    "GEOMETRIC_TOLERANCE",
    "ANGULARITY_TOLERANCE",
    "CIRCULAR_RUNOUT_TOLERANCE",
    "COAXIALITY_TOLERANCE",
    "CONCENTRICITY_TOLERANCE",
    "CYLINDRICITY_TOLERANCE",
    "FLATNESS_TOLERANCE",
    "GEOMETRIC_TOLERANCE_WITH_DATUM_REFERENCE",
    "GEOMETRIC_TOLERANCE_WITH_DEFINED_AREA_UNIT",
    "GEOMETRIC_TOLERANCE_WITH_DEFINED_UNIT",
    "GEOMETRIC_TOLERANCE_WITH_MAXIMUM_TOLERANCE",
    "GEOMETRIC_TOLERANCE_WITH_MODIFIERS",
    "LINE_PROFILE_TOLERANCE",
    "PARALLELISM_TOLERANCE",
    "PERPENDICULARITY_TOLERANCE",
    "POSITION_TOLERANCE",
    "ROUNDNESS_TOLERANCE",
    "STRAIGHTNESS_TOLERANCE",
    "SURFACE_PROFILE_TOLERANCE",
    "SYMMETRY_TOLERANCE",
    "TOTAL_RUNOUT_TOLERANCE",
    "UNEQUALLY_DISPOSED_GEOMETRIC_TOLERANCE",
];

/// The closure rule table. Positions are those of the long-form schemas
/// stepq is tested against; `tests/schemas.rs` checks every one of them.
pub const RULES: &[Rule] = &[
    // Shapes and properties of what is extracted.
    owned(
        &["PROPERTY_DEFINITION", "PRODUCT_DEFINITION_SHAPE"],
        "definition",
        2,
        Some(("PROPERTY_DEFINITION", 2)),
    ),
    owned(
        &[
            "PROPERTY_DEFINITION_REPRESENTATION",
            "SHAPE_DEFINITION_REPRESENTATION",
        ],
        "definition",
        0,
        Some(("PROPERTY_DEFINITION_REPRESENTATION", 0)),
    ),
    // A shape spread over several representations. Complex instances are
    // excluded: those are assembly placements, which reach the parent.
    owned(&["SHAPE_REPRESENTATION_RELATIONSHIP"], "rep_1", 2, None),
    owned(&["SHAPE_REPRESENTATION_RELATIONSHIP"], "rep_2", 3, None),
    owned(
        &["GENERAL_PROPERTY_ASSOCIATION"],
        "derived_definition",
        3,
        Some(("GENERAL_PROPERTY_ASSOCIATION", 3)),
    ),
    // Assemblies: from a parent to its child usages, never upwards.
    owned(
        &[
            "NEXT_ASSEMBLY_USAGE_OCCURRENCE",
            "QUANTIFIED_ASSEMBLY_COMPONENT_USAGE",
        ],
        "relating_product_definition",
        3,
        Some(("PRODUCT_DEFINITION_RELATIONSHIP", 3)),
    ),
    owned(
        &["CONTEXT_DEPENDENT_SHAPE_REPRESENTATION"],
        "represented_product_relation",
        1,
        Some(("CONTEXT_DEPENDENT_SHAPE_REPRESENTATION", 1)),
    ),
    // Presentation.
    owned(
        &["STYLED_ITEM", "OVER_RIDING_STYLED_ITEM"],
        "item",
        2,
        Some(("STYLED_ITEM", 1)),
    ),
    // PMI presentation: the plane an annotation is placed on belongs to the
    // callouts it holds, and a saved view (camera) and the draughting model
    // that links presentation to semantic PMI belong to the draughting model
    // those callouts are extracted with.
    owned(
        &["ANNOTATION_PLANE"],
        "elements",
        3,
        Some(("ANNOTATION_PLANE", 0)),
    ),
    owned(
        &["MECHANICAL_DESIGN_AND_DRAUGHTING_RELATIONSHIP"],
        "rep_2",
        3,
        Some(("REPRESENTATION_RELATIONSHIP", 3)),
    ),
    owned(
        &["MODEL_GEOMETRIC_VIEW"],
        "rep",
        3,
        Some(("CHARACTERIZED_ITEM_WITHIN_REPRESENTATION", 1)),
    ),
    // Validation properties of one annotation (AP242: its text, curve
    // length, number of points) hang off this link to the callout.
    owned(
        &["CHARACTERIZED_ITEM_WITHIN_REPRESENTATION"],
        "item",
        2,
        Some(("CHARACTERIZED_ITEM_WITHIN_REPRESENTATION", 0)),
    ),
    // Saved views (Creo): a product's presentation set, through its
    // presentation areas, holds views of the product's draughting model.
    // Each set, area and view belongs to one product.
    owned(
        &["PRESENTED_ITEM_REPRESENTATION"],
        "item",
        1,
        Some(("PRESENTED_ITEM_REPRESENTATION", 1)),
    ),
    owned(&["AREA_IN_SET"], "in_set", 1, Some(("AREA_IN_SET", 1))),
    owned(
        &["PRESENTATION_SIZE"],
        "unit",
        0,
        Some(("PRESENTATION_SIZE", 0)),
    ),
    // Supplemental geometry (datum planes, axes, sketches) of a shape.
    owned(
        &["CONSTRUCTIVE_GEOMETRY_REPRESENTATION_RELATIONSHIP"],
        "rep_1",
        2,
        Some(("REPRESENTATION_RELATIONSHIP", 2)),
    ),
    // PMI: shape aspects, dimensions, tolerances and their associations.
    owned(SHAPE_ASPECTS, "of_shape", 2, Some(("SHAPE_ASPECT", 2))),
    owned(
        SHAPE_ASPECT_RELATIONSHIPS,
        "relating_shape_aspect",
        2,
        Some(("SHAPE_ASPECT_RELATIONSHIP", 2)),
    ),
    owned(
        SHAPE_ASPECT_RELATIONSHIPS,
        "related_shape_aspect",
        3,
        Some(("SHAPE_ASPECT_RELATIONSHIP", 3)),
    ),
    owned(
        // Not DIMENSIONAL_SIZE_WITH_DATUM_FEATURE: its other supertype puts
        // `name` first in a simple instance.
        &[
            "DIMENSIONAL_SIZE",
            "ANGULAR_SIZE",
            "DIMENSIONAL_SIZE_WITH_PATH",
        ],
        "applies_to",
        0,
        Some(("DIMENSIONAL_SIZE", 0)),
    ),
    owned(
        &["DIMENSIONAL_CHARACTERISTIC_REPRESENTATION"],
        "dimension",
        0,
        Some(("DIMENSIONAL_CHARACTERISTIC_REPRESENTATION", 0)),
    ),
    // How a PMI value is displayed (decimal places, format).
    owned(
        &["MEASURE_QUALIFICATION"],
        "qualified_measure",
        2,
        Some(("MEASURE_QUALIFICATION", 2)),
    ),
    owned(
        &["DRAUGHTING_CALLOUT_RELATIONSHIP"],
        "relating_draughting_callout",
        2,
        Some(("DRAUGHTING_CALLOUT_RELATIONSHIP", 2)),
    ),
    // Metadata owned by what it describes.
    owned(
        &["NAME_ATTRIBUTE"],
        "named_item",
        1,
        Some(("NAME_ATTRIBUTE", 1)),
    ),
    // Persistent identifiers (AP242 PMI) and descriptions of shape aspects,
    // datums and tolerances: left behind by every split before.
    owned(
        &["ID_ATTRIBUTE"],
        "identified_item",
        1,
        Some(("ID_ATTRIBUTE", 1)),
    ),
    owned(
        &["DESCRIPTION_ATTRIBUTE"],
        "described_item",
        1,
        Some(("DESCRIPTION_ATTRIBUTE", 1)),
    ),
    // An address belongs to the people or organizations it lists.
    owned(
        &["PERSONAL_ADDRESS"],
        "people",
        12,
        Some(("PERSONAL_ADDRESS", 0)),
    ),
    owned(
        &["ORGANIZATIONAL_ADDRESS"],
        "organizations",
        12,
        Some(("ORGANIZATIONAL_ADDRESS", 0)),
    ),
    owned(
        &["APPROVAL_DATE_TIME"],
        "dated_approval",
        1,
        Some(("APPROVAL_DATE_TIME", 1)),
    ),
    owned(
        &["APPROVAL_PERSON_ORGANIZATION"],
        "authorized_approval",
        1,
        Some(("APPROVAL_PERSON_ORGANIZATION", 1)),
    ),
    owned(
        GEOMETRIC_TOLERANCES,
        "toleranced_shape_aspect",
        3,
        Some(("GEOMETRIC_TOLERANCE", 3)),
    ),
    owned(
        &["PLUS_MINUS_TOLERANCE"],
        "toleranced_dimension",
        1,
        Some(("PLUS_MINUS_TOLERANCE", 1)),
    ),
    owned(
        &[
            "ITEM_IDENTIFIED_REPRESENTATION_USAGE",
            "DRAUGHTING_MODEL_ITEM_ASSOCIATION",
            "DRAUGHTING_MODEL_ITEM_ASSOCIATION_WITH_PLACEHOLDER",
            "GEOMETRIC_ITEM_SPECIFIC_USAGE",
        ],
        "definition",
        2,
        Some(("ITEM_IDENTIFIED_REPRESENTATION_USAGE", 2)),
    ),
    // File-level context every output needs.
    owned(
        &["APPLICATION_PROTOCOL_DEFINITION"],
        "application",
        3,
        Some(("APPLICATION_PROTOCOL_DEFINITION", 3)),
    ),
    owned(
        &["PRODUCT_CATEGORY_RELATIONSHIP"],
        "sub_category",
        3,
        Some(("PRODUCT_CATEGORY_RELATIONSHIP", 3)),
    ),
    // Shared aggregates: filtered, never copied wholesale.
    shared(
        &["PRODUCT_RELATED_PRODUCT_CATEGORY"],
        "products",
        2,
        Some(("PRODUCT_RELATED_PRODUCT_CATEGORY", 0)),
    ),
    shared(
        &["PRESENTATION_LAYER_ASSIGNMENT"],
        "assigned_items",
        2,
        Some(("PRESENTATION_LAYER_ASSIGNMENT", 2)),
    ),
    shared(
        &[
            "MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION",
            "DRAUGHTING_MODEL",
        ],
        "items",
        1,
        Some(("REPRESENTATION", 1)),
    ),
    shared(
        &["APPLIED_PRESENTED_ITEM"],
        "items",
        0,
        Some(("APPLIED_PRESENTED_ITEM", 0)),
    ),
    shared(
        &["INVISIBILITY"],
        "invisible_items",
        0,
        Some(("INVISIBILITY", 0)),
    ),
    shared(
        &[
            "APPLIED_PERSON_AND_ORGANIZATION_ASSIGNMENT",
            "APPLIED_DATE_AND_TIME_ASSIGNMENT",
            "APPLIED_DATE_ASSIGNMENT",
            "APPLIED_ORGANIZATION_ASSIGNMENT",
            "APPLIED_DOCUMENT_REFERENCE",
            "APPLIED_IDENTIFICATION_ASSIGNMENT",
            "APPLIED_CLASSIFICATION_ASSIGNMENT",
            "CC_DESIGN_PERSON_AND_ORGANIZATION_ASSIGNMENT",
            "CC_DESIGN_DATE_AND_TIME_ASSIGNMENT",
            "CC_DESIGN_SPECIFICATION_REFERENCE",
        ],
        "items",
        2,
        None,
    ),
    shared(
        &[
            "APPLIED_APPROVAL_ASSIGNMENT",
            "APPLIED_SECURITY_CLASSIFICATION_ASSIGNMENT",
            "APPLIED_GROUP_ASSIGNMENT",
            "CC_DESIGN_APPROVAL",
            "CC_DESIGN_SECURITY_CLASSIFICATION",
            "CC_DESIGN_CERTIFICATION",
            "CC_DESIGN_CONTRACT",
        ],
        "items",
        1,
        None,
    ),
];

/// The instances that make up one extracted product definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extraction {
    nodes: Vec<usize>,
    pruned: Vec<usize>,
}

impl Extraction {
    /// The extracted nodes, in ascending order.
    pub fn nodes(&self) -> &[usize] {
        &self.nodes
    }

    /// The extracted nodes that are shared aggregates: when written, their
    /// lists must drop items that are not extracted. See
    /// [`Writer::write_pruned`](crate::p21::Writer::write_pruned).
    pub fn pruned(&self) -> &[usize] {
        &self.pruned
    }

    /// True if `node` is extracted.
    pub fn contains(&self, node: usize) -> bool {
        self.nodes.binary_search(&node).is_ok()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Out,
    Owned,
    Shared(&'static Rule),
    Excluded,
}

/// Extracts `seeds` — usually one product definition — with everything that
/// belongs to them under [`RULES`].
///
/// # Panics
///
/// Panics if a seed is not a node of `graph`.
pub fn extract(graph: &Graph<'_>, seeds: &[usize]) -> Extraction {
    extract_excluding(graph, seeds, &[])
}

/// Like [`extract`], but never takes the `excluded` nodes, and so nothing
/// that only they would bring in. Every extracted instance that refers to an
/// excluded node is pruned as well, so that
/// [`Writer::write_pruned`](crate::p21::Writer::write_pruned) drops those
/// list items; a reference to an excluded node that is not a list item makes
/// that write fail rather than leave the output dangling.
///
/// `stepq split --bodies` writes one solid of a multi-body part this way:
/// the part, with its other solids excluded.
///
/// # Panics
///
/// Panics if a seed or an excluded node is not a node of `graph`.
pub fn extract_excluding(graph: &Graph<'_>, seeds: &[usize], excluded: &[usize]) -> Extraction {
    let exchange = graph.exchange();
    let mut state = vec![State::Out; graph.len()];
    for &node in excluded {
        state[node] = State::Excluded;
    }
    let mut queue = VecDeque::new();
    let take = |node: usize, state: &mut [State], queue: &mut VecDeque<usize>| {
        if state[node] == State::Out {
            state[node] =
                shared_rule(exchange, graph.instance(node)).map_or(State::Owned, State::Shared);
            queue.push_back(node);
        }
    };
    for &seed in seeds {
        take(seed, &mut state, &mut queue);
    }

    let mut targets = Vec::new();
    while let Some(node) = queue.pop_front() {
        let instance = graph.instance(node);
        match state[node] {
            State::Shared(rule) => {
                targets.clear();
                references_outside(exchange, instance, rule, &mut targets);
                for &id in &targets {
                    if let Some(target) = graph.node(id) {
                        take(target, &mut state, &mut queue);
                    }
                }
            }
            _ => {
                for &target in graph.references(node) {
                    take(target, &mut state, &mut queue);
                }
            }
        }
        for &referrer in graph.referenced_by(node) {
            if state[referrer] == State::Out
                && matching_rule(exchange, graph.instance(referrer), instance.id).is_some()
            {
                take(referrer, &mut state, &mut queue);
            }
        }
    }

    let mut nodes = Vec::new();
    let mut pruned = Vec::new();
    let state_of = &state;
    for (node, state) in state.iter().enumerate() {
        match state {
            State::Out | State::Excluded => {}
            State::Owned => {
                nodes.push(node);
                let lists_excluded = || {
                    graph
                        .references(node)
                        .iter()
                        .any(|&target| state_of[target] == State::Excluded)
                };
                if !excluded.is_empty() && lists_excluded() {
                    pruned.push(node);
                }
            }
            State::Shared(_) => {
                nodes.push(node);
                pruned.push(node);
            }
        }
    }
    Extraction { nodes, pruned }
}

/// The nodes that none of `extractions` contains, in ascending order:
/// what a split would leave behind.
pub fn orphans(graph: &Graph<'_>, extractions: &[Extraction]) -> Vec<usize> {
    let mut taken = vec![false; graph.len()];
    for extraction in extractions {
        for &node in extraction.nodes() {
            taken[node] = true;
        }
    }
    (0..graph.len()).filter(|&node| !taken[node]).collect()
}

/// The rule under which `instance` would join an extraction containing
/// instance `#target`, if any.
fn matching_rule(
    exchange: &Exchange<'_>,
    instance: &Instance,
    target: u64,
) -> Option<&'static Rule> {
    RULES.iter().find(|rule| {
        rule_param(exchange, instance, rule).is_some_and(|param| {
            let mut ids = Vec::new();
            collect_references(&param, &mut ids);
            ids.contains(&target)
        })
    })
}

/// The shared rule `instance` is an aggregate of, if any.
fn shared_rule(exchange: &Exchange<'_>, instance: &Instance) -> Option<&'static Rule> {
    RULES
        .iter()
        .filter(|rule| rule.kind == RuleKind::Shared)
        .find(|rule| rule_record(exchange, instance, rule).is_some())
}

/// The record holding `rule`'s attribute in `instance` and the attribute's
/// position in it, if the rule applies to the instance.
fn rule_record<'e>(
    exchange: &'e Exchange<'_>,
    instance: &Instance,
    rule: &Rule,
) -> Option<(Record<'e>, usize)> {
    let mut records = exchange.records(instance);
    if instance.is_complex() {
        let (partial, index) = rule.partial?;
        let has_entity = records
            .clone()
            .any(|record| rule.entities.iter().any(|entity| record.is(entity)));
        if !has_entity {
            return None;
        }
        records
            .find(|record| record.is(partial))
            .map(|record| (record, index))
    } else {
        let record = records.next()?;
        rule.entities
            .iter()
            .any(|entity| record.is(entity))
            .then_some((record, rule.simple))
    }
}

fn rule_param<'e>(
    exchange: &'e Exchange<'_>,
    instance: &Instance,
    rule: &Rule,
) -> Option<Param<'e>> {
    let (record, index) = rule_record(exchange, instance, rule)?;
    record.param(index)
}

/// Every reference in `instance` except those inside `rule`'s attribute.
fn references_outside(
    exchange: &Exchange<'_>,
    instance: &Instance,
    rule: &Rule,
    out: &mut Vec<u64>,
) {
    let skip = rule_record(exchange, instance, rule);
    for record in exchange.records(instance) {
        for (index, param) in record.params().enumerate() {
            let skipped = skip.is_some_and(|(held, position)| {
                position == index && held.name().eq_ignore_ascii_case(record.name())
            });
            if !skipped {
                collect_references(&param, out);
            }
        }
    }
}

fn collect_references(param: &Param<'_>, out: &mut Vec<u64>) {
    match param {
        Param::Reference(id) => out.push(*id),
        Param::List(items) => {
            for item in items.clone() {
                collect_references(&item, out);
            }
        }
        Param::Typed(record) => {
            for item in record.params() {
                collect_references(&item, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::parse;

    /// Assembly A (#10) with one usage of part C (#30). C has a shape, a
    /// styled solid on a shared layer, and a category shared with A.
    const SAMPLE: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('AP214'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('design');
#2=APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2001,#1);
#3=PRODUCT_CONTEXT('',#1,'mechanical');
#4=PRODUCT_DEFINITION_CONTEXT('part definition',#1,'design');
#11=PRODUCT('A','assembly','',(#3));
#12=PRODUCT_DEFINITION_FORMATION('1','',#11);
#10=PRODUCT_DEFINITION('a','',#12,#4);
#31=PRODUCT('C','bolt','',(#3));
#32=PRODUCT_DEFINITION_FORMATION('1','',#31);
#30=PRODUCT_DEFINITION('c','',#32,#4);
#40=PRODUCT_RELATED_PRODUCT_CATEGORY('part','',(#11,#31));
#50=SHAPE_REPRESENTATION('a',(#60),#90);
#52=SHAPE_REPRESENTATION('c',(#60,#53),#90);
#53=MANIFOLD_SOLID_BREP('solid',#54);
#54=CLOSED_SHELL('',());
#60=AXIS2_PLACEMENT_3D('',#61,$,$);
#61=CARTESIAN_POINT('',(0.,0.,0.));
#70=PRODUCT_DEFINITION_SHAPE('','',#10);
#72=PRODUCT_DEFINITION_SHAPE('','',#30);
#80=SHAPE_DEFINITION_REPRESENTATION(#70,#50);
#82=SHAPE_DEFINITION_REPRESENTATION(#72,#52);
#90=GEOMETRIC_REPRESENTATION_CONTEXT(3);
#100=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u1','C1','',#10,#30,$);
#101=PRODUCT_DEFINITION_SHAPE('','',#100);
#102=(REPRESENTATION_RELATIONSHIP('','',#52,#50)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#103)SHAPE_REPRESENTATION_RELATIONSHIP());
#103=ITEM_DEFINED_TRANSFORMATION('','',#60,#60);
#104=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#102,#101);
#110=STYLED_ITEM('',(#111),#53);
#111=PRESENTATION_STYLE_ASSIGNMENT(());
#120=PRESENTATION_LAYER_ASSIGNMENT('layer 1','',(#53,#61));
#130=MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION('',(#110),#90);
ENDSEC;END-ISO-10303-21;";

    fn ids(graph: &Graph<'_>, nodes: &[usize]) -> Vec<u64> {
        let mut ids: Vec<u64> = nodes.iter().map(|&n| graph.instance(n).id).collect();
        ids.sort_unstable();
        ids
    }

    fn graph() -> Graph<'static> {
        Graph::new(parse(SAMPLE.as_bytes()).unwrap()).unwrap()
    }

    #[test]
    fn a_part_takes_its_shape_style_and_filtered_aggregates_but_not_its_parent() {
        let graph = graph();
        let part = graph.node(30).unwrap();
        let extraction = extract(&graph, &[part]);
        assert_eq!(
            ids(&graph, extraction.nodes()),
            [
                1, 2, 3, 4, 30, 31, 32, 40, 52, 53, 54, 60, 61, 72, 82, 90, 110, 111, 120, 130
            ]
        );
        assert_eq!(ids(&graph, extraction.pruned()), [40, 120, 130]);
        for parent_side in [10, 11, 100, 102, 104] {
            assert!(
                !extraction.contains(graph.node(parent_side).unwrap()),
                "#{parent_side}"
            );
        }
    }

    #[test]
    fn an_assembly_takes_its_usages_placements_and_components() {
        let graph = graph();
        let assembly = graph.node(10).unwrap();
        let extraction = extract(&graph, &[assembly]);
        let everything: Vec<u64> = graph.exchange().instances().iter().map(|i| i.id).collect();
        let mut everything = everything;
        everything.sort_unstable();
        assert_eq!(ids(&graph, extraction.nodes()), everything);
        assert!(orphans(&graph, &[extraction]).is_empty());
    }

    #[test]
    fn excluded_nodes_and_what_only_they_bring_are_left_out() {
        let graph = graph();
        let part = graph.node(30).unwrap();
        let solid = graph.node(53).unwrap();
        let extraction = extract_excluding(&graph, &[part], &[solid]);
        // The solid, its shell and its style are gone …
        for gone in [53, 54, 110, 111] {
            assert!(!extraction.contains(graph.node(gone).unwrap()), "#{gone}");
        }
        // … as is the presentation list #130, reached only through that
        // style. The representation that listed the solid stays, pruned, as
        // does the layer, whose other item is still extracted.
        assert_eq!(
            ids(&graph, extraction.nodes()),
            [1, 2, 3, 4, 30, 31, 32, 40, 52, 60, 61, 72, 82, 90, 120]
        );
        assert_eq!(ids(&graph, extraction.pruned()), [40, 52, 120]);
        assert_eq!(
            extract_excluding(&graph, &[part], &[]),
            extract(&graph, &[part])
        );
    }

    #[test]
    fn shared_aggregates_are_not_followed_through_their_list() {
        let graph = graph();
        // The category lists both products; extracting C must not pull in A.
        let extraction = extract(&graph, &[graph.node(30).unwrap()]);
        assert!(extraction.contains(graph.node(40).unwrap()));
        assert!(!extraction.contains(graph.node(11).unwrap()));
    }

    #[test]
    fn saved_views_and_addresses_go_with_their_product() {
        // Part C with a saved view (presentation set, area) and a person
        // with an address assigned to it; colour #400 is used by nothing.
        let src = "ISO-10303-21;HEADER;FILE_SCHEMA(('AP214'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('design');
#3=PRODUCT_CONTEXT('',#1,'mechanical');
#4=PRODUCT_DEFINITION_CONTEXT('part definition',#1,'design');
#31=PRODUCT('C','bolt','',(#3));
#32=PRODUCT_DEFINITION_FORMATION('1','',#31);
#30=PRODUCT_DEFINITION('c','',#32,#4);
#90=GEOMETRIC_REPRESENTATION_CONTEXT(2);
#200=APPLIED_PRESENTED_ITEM((#30));
#201=PRESENTATION_SET();
#202=PRESENTED_ITEM_REPRESENTATION(#201,#200);
#203=PRESENTATION_AREA('',(#205),#90);
#204=AREA_IN_SET(#203,#201);
#205=PLANAR_BOX('',1.,1.,#206);
#206=AXIS2_PLACEMENT_2D('',#207,$);
#207=CARTESIAN_POINT('',(0.,0.));
#300=PERSON('p','Smith',$,$,$,$);
#301=PERSONAL_ADDRESS($,$,$,$,$,$,$,'US',$,$,$,$,(#300),$);
#302=ORGANIZATION('o','Org',$);
#303=PERSON_AND_ORGANIZATION(#300,#302);
#304=PERSON_AND_ORGANIZATION_ROLE('creator');
#305=APPLIED_PERSON_AND_ORGANIZATION_ASSIGNMENT(#303,#304,(#30));
#400=COLOUR_RGB('',0.,0.,1.);
ENDSEC;END-ISO-10303-21;";
        let graph = Graph::new(parse(src.as_bytes()).unwrap()).unwrap();
        let extraction = extract(&graph, &[graph.node(30).unwrap()]);
        assert_eq!(
            ids(&graph, extraction.nodes()),
            [
                1, 3, 4, 30, 31, 32, 90, 200, 201, 202, 203, 204, 205, 206, 207, 300, 301, 302,
                303, 304, 305
            ]
        );
        assert_eq!(ids(&graph, extraction.pruned()), [200, 305]);
        assert_eq!(ids(&graph, &orphans(&graph, &[extraction])), [400]);
    }

    #[test]
    fn orphans_are_what_no_extraction_takes() {
        let graph = graph();
        let part = extract(&graph, &[graph.node(30).unwrap()]);
        let orphaned = ids(&graph, &orphans(&graph, &[part]));
        assert_eq!(orphaned, [10, 11, 12, 50, 70, 80, 100, 101, 102, 103, 104]);
    }

    #[test]
    fn rules_are_well_formed() {
        for rule in RULES {
            assert!(!rule.entities.is_empty());
            assert!(
                rule.entities
                    .iter()
                    .all(|e| e.chars().all(|c| c.is_ascii_uppercase() || c == '_'))
            );
            assert!(
                rule.attribute
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
            );
        }
    }
}
