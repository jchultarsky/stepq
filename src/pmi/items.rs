//! The representation items shape aspects stand for: from a tolerance's,
//! dimension's or datum's shape aspect to the faces, edges and points the
//! file names for it. See [`AspectItems`].

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

use crate::model::Graph;
use crate::p21::{Exchange, Instance, Param, Record};
use crate::record::{self, text};

/// Aspects more than this many relationships from the definition, by the
/// shortest route, are not reached ([`UnresolvedReason::TooDeep`]). The
/// longest route in the fixtures has two.
const MAX_DEPTH: usize = 64;

/// `shape_aspect_relationship` and its subtypes, in the four schemas.
const RELATIONSHIPS: &[&str] = &[
    "SHAPE_ASPECT_RELATIONSHIP",
    "ANGULAR_LOCATION",
    "ASSEMBLY_JOINT",
    "ASSEMBLY_SHAPE_CONSTRAINT_ITEM_RELATIONSHIP",
    "ASSEMBLY_SHAPE_JOINT_ITEM_RELATIONSHIP",
    "COMPONENT_FEATURE_JOINT",
    "COMPONENT_FEATURE_RELATIONSHIP",
    "CONNECTION_ZONE_BASED_ASSEMBLY_JOINT",
    "CONNECTION_ZONE_INTERFACE_PLANE_RELATIONSHIP",
    "CONNECTIVITY_DEFINITION_ITEM_RELATIONSHIP",
    "CONTACT_FEATURE_FIT_RELATIONSHIP",
    "DIMENSIONAL_LOCATION",
    "DIMENSIONAL_LOCATION_WITH_DATUM_FEATURE",
    "DIMENSIONAL_LOCATION_WITH_NORMAL_CONSTRAINT",
    "DIMENSIONAL_LOCATION_WITH_PATH",
    "DIRECTED_DIMENSIONAL_LOCATION",
    "FEATURE_COMPONENT_RELATIONSHIP",
    "FEATURE_FOR_DATUM_TARGET_RELATIONSHIP",
    "MAKE_FROM_FEATURE_RELATIONSHIP",
    "PATTERN_OFFSET_MEMBERSHIP",
    "PATTERN_OMIT_MEMBERSHIP",
    "SHAPE_ASPECT_ASSOCIATIVITY",
    "SHAPE_ASPECT_DERIVING_RELATIONSHIP",
    "SHAPE_ASPECT_TRANSITION",
    "SHAPE_DEFINING_RELATIONSHIP",
    "SHAPE_FEATURE_DEFINITION_ELEMENT_RELATIONSHIP",
    "SHAPE_FEATURE_FIT_RELATIONSHIP",
];

/// Simple relationship records whose `shape_aspect_relationship` attributes
/// do not come first: the number that do. Every other subtype keeps
/// `relating_shape_aspect` and `related_shape_aspect` as attributes 2 and 3.
const RELATIONSHIP_OFFSETS: &[(&str, usize)] = &[
    ("CONNECTION_ZONE_INTERFACE_PLANE_RELATIONSHIP", 4),
    ("DIMENSIONAL_LOCATION_WITH_DATUM_FEATURE", 4),
];

/// `dimensional_location` and its subtypes: dimensions, which `pmi` reads,
/// not links between aspects.
const LOCATIONS: &[&str] = &[
    "ANGULAR_LOCATION",
    "DIMENSIONAL_LOCATION",
    "DIMENSIONAL_LOCATION_WITH_DATUM_FEATURE",
    "DIMENSIONAL_LOCATION_WITH_NORMAL_CONSTRAINT",
    "DIMENSIONAL_LOCATION_WITH_PATH",
    "DIRECTED_DIMENSIONAL_LOCATION",
];

/// `dimensional_size` and its subtypes.
const SIZES: &[&str] = &[
    "ANGULAR_SIZE",
    "DIMENSIONAL_SIZE",
    "DIMENSIONAL_SIZE_WITH_DATUM_FEATURE",
    "DIMENSIONAL_SIZE_WITH_PATH",
    "EXTERNALLY_DEFINED_DIMENSION_DEFINITION",
];

/// `item_identified_representation_usage` and its subtypes.
const USAGES: &[&str] = &[
    "ITEM_IDENTIFIED_REPRESENTATION_USAGE",
    "CHAIN_BASED_GEOMETRIC_ITEM_SPECIFIC_USAGE",
    "CHAIN_BASED_ITEM_IDENTIFIED_REPRESENTATION_USAGE",
    "DRAUGHTING_MODEL_ITEM_ASSOCIATION",
    "DRAUGHTING_MODEL_ITEM_ASSOCIATION_WITH_EXTERNAL_IMAGE_PLACEMENT",
    "DRAUGHTING_MODEL_ITEM_ASSOCIATION_WITH_PLACEHOLDER",
    "GEOMETRIC_ITEM_SPECIFIC_USAGE",
    "MECHANICAL_DESIGN_REQUIREMENT_ITEM_ASSOCIATION",
    "PMI_REQUIREMENT_ITEM_ASSOCIATION",
];

/// `draughting_model_item_association` and its subtypes: presentation.
const PRESENTATION: &[&str] = &[
    "DRAUGHTING_MODEL_ITEM_ASSOCIATION",
    "DRAUGHTING_MODEL_ITEM_ASSOCIATION_WITH_EXTERNAL_IMAGE_PLACEMENT",
    "DRAUGHTING_MODEL_ITEM_ASSOCIATION_WITH_PLACEHOLDER",
];

/// `composite_shape_aspect` and its subtypes: their components are the
/// related sides of their plain relationships.
const COMPOSITES: &[&str] = &[
    "ALL_AROUND_SHAPE_ASPECT",
    "BETWEEN_SHAPE_ASPECT",
    "COMMON_DATUM",
    "COMPOSITE_GROUP_SHAPE_ASPECT",
    "COMPOSITE_SHAPE_ASPECT",
    "COMPOSITE_UNIT_SHAPE_ASPECT",
    "CONTINUOUS_SHAPE_ASPECT",
];

/// `derived_shape_aspect` and its subtypes.
const DERIVED: &[&str] = &[
    "APEX",
    "CENTRE_OF_SYMMETRY",
    "DERIVED_SHAPE_ASPECT",
    "EXTENSION",
    "GEOMETRIC_ALIGNMENT",
    "GEOMETRIC_CONTACT",
    "GEOMETRIC_INTERSECTION",
    "PARALLEL_OFFSET",
    "PERPENDICULAR_TO",
    "TANGENT",
];

/// `datum_feature` and its subtypes.
const DATUM_FEATURES: &[&str] = &[
    "DATUM_FEATURE",
    "DIMENSIONAL_LOCATION_WITH_DATUM_FEATURE",
    "DIMENSIONAL_SIZE_WITH_DATUM_FEATURE",
];

/// `datum_target` and its subtypes.
const DATUM_TARGETS: &[&str] = &["DATUM_TARGET", "PLACED_DATUM_TARGET_FEATURE"];

/// `datum` and its subtypes.
const DATUMS: &[&str] = &["COMMON_DATUM", "DATUM"];

/// How a route reaches an aspect from the one before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum StepKind {
    /// From a composite (`composite_shape_aspect` and its subtypes,
    /// `common_datum` included) to a component, by a plain
    /// `shape_aspect_relationship`: the value applies to the group, not to
    /// each member.
    Component,
    /// From a derived aspect (`centre_of_symmetry`, `apex`, …) to one it is
    /// derived from, by a `shape_aspect_deriving_relationship`: the faces an
    /// axis or a centre plane is derived from, not what is toleranced.
    DerivedFrom,
    /// From a datum to a datum feature whose plain relationship names it
    /// (the schema's `datum_feature` WR1).
    DatumFeature,
    /// From a datum to a datum target whose plain relationship names it.
    DatumTarget,
    /// From a datum target to the feature it lies on, by a
    /// `feature_for_datum_target_relationship`.
    TargetFeature,
}

impl StepKind {
    const ALL: [Self; 5] = [
        Self::Component,
        Self::DerivedFrom,
        Self::DatumFeature,
        Self::DatumTarget,
        Self::TargetFeature,
    ];

    fn bit(self) -> u8 {
        match self {
            Self::Component => 1,
            Self::DerivedFrom => 2,
            Self::DatumFeature => 4,
            Self::DatumTarget => 8,
            Self::TargetFeature => 16,
        }
    }
}

/// A set of [`StepKind`]s: the kinds of step one route takes. The empty set
/// is a usage naming the item directly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StepKinds(u8);

impl StepKinds {
    /// No kind: a direct usage.
    pub const NONE: Self = Self(0);
    /// Every kind.
    pub const ALL: Self = Self(31);

    /// This set with `kind` added.
    #[must_use]
    pub fn with(self, kind: StepKind) -> Self {
        Self(self.0 | kind.bit())
    }

    /// True if `kind` is in the set.
    pub fn contains(self, kind: StepKind) -> bool {
        self.0 & kind.bit() != 0
    }

    /// True if every kind in this set is in `other`.
    pub fn is_subset(self, other: Self) -> bool {
        self.0 & !other.0 == 0
    }

    /// True if the set is empty: a direct usage.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The kinds in the set, in [`StepKind`] order.
    pub fn iter(self) -> impl Iterator<Item = StepKind> {
        StepKind::ALL
            .into_iter()
            .filter(move |&kind| self.contains(kind))
    }
}

impl FromIterator<StepKind> for StepKinds {
    fn from_iter<I: IntoIterator<Item = StepKind>>(kinds: I) -> Self {
        kinds.into_iter().fold(Self::NONE, Self::with)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for StepKinds {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}

/// The distinct [`StepKinds`] of a set of routes, at most 32 (one per subset
/// of the five kinds), as a bit per subset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Routes(u32);

impl Routes {
    const DIRECT: Self = Self(1);

    /// Every route here, extended by a step of `kind`.
    fn then(self, kind: StepKind) -> Self {
        let mut extended = 0;
        for subset in 0..32u8 {
            if self.0 & (1 << subset) != 0 {
                extended |= 1 << (subset | kind.bit());
            }
        }
        Self(extended)
    }

    fn sets(self) -> Vec<StepKinds> {
        (0..32u8)
            .filter(|subset| self.0 & (1 << subset) != 0)
            .map(StepKinds)
            .collect()
    }
}

/// One `item_identified_representation_usage`, of any subtype.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ItemUsage {
    /// The usage's `#id`.
    pub instance: u64,
    /// Its kind, and whether [`AspectItems::resolve`] follows it.
    pub kind: ItemUsageKind,
    /// Its upper-cased entity types, joined with `+`.
    pub entity: String,
    /// `name`.
    pub name: Option<String>,
    /// `description`.
    pub description: Option<String>,
    /// `definition`: what its items stand for.
    pub definition: u64,
    /// `used_representation` as written; `None` if it is not a reference.
    /// stepq does not check that it lists the items (the schema's WR1): 510
    /// of the 2,587 items the NIST files identify lie in a representation
    /// related to it instead.
    pub used_representation: Option<u64>,
    /// `identified_item`: one item, or the members of a typed
    /// `set_representation_item` or `list_representation_item`, in order;
    /// empty if it is neither.
    pub items: Vec<u64>,
}

/// What an [`ItemUsage`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum ItemUsageKind {
    /// A simple `geometric_item_specific_usage`. Followed.
    GeometricItemSpecific,
    /// A simple `item_identified_representation_usage` of no subtype.
    /// Followed.
    ItemIdentified,
    /// A `draughting_model_item_association`, of any subtype: it ties
    /// semantic PMI to its presentation. Listed, never followed.
    Presentation,
    /// Any other subtype (a chain-based usage, which needs an assembly path,
    /// a requirement association), a complex instance, or a usage whose
    /// `identified_item` is neither a reference nor a typed set or list of
    /// them. Reported, never followed.
    Other,
}

/// One step of a route: a relationship followed to an aspect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Step {
    /// The relationship's `#id`.
    pub relationship: u64,
    /// What kind of step it is.
    pub kind: StepKind,
    /// The aspect it leads to.
    pub aspect: u64,
}

/// What [`AspectItems::resolve`] found for one definition.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Resolution {
    /// The `#id` asked about.
    pub definition: u64,
    /// Every item reached, each once, in the order found (nearest first).
    pub items: Vec<ResolvedItem>,
    /// Every point where a route stopped short of an item, with the reason.
    pub unresolved: Vec<Unresolved>,
}

impl Resolution {
    /// True if no route stopped short: [`items`](Self::items) are all the
    /// definition stands for.
    pub fn is_complete(&self) -> bool {
        self.unresolved.is_empty()
    }

    /// The `#id`s of the items some route reaches using only steps of the
    /// `allowed` kinds, each once, in the order found. Items a usage names
    /// directly are always included; `StepKinds::NONE` gives only those.
    pub fn item_ids(&self, allowed: StepKinds) -> Vec<u64> {
        self.items
            .iter()
            .filter(|item| {
                item.route_kinds
                    .iter()
                    .any(|kinds| kinds.is_subset(allowed))
            })
            .map(|item| item.item)
            .collect()
    }
}

/// One item a definition stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ResolvedItem {
    /// The item's `#id`.
    pub item: u64,
    /// Its upper-cased entity types, joined with `+`, such as
    /// `ADVANCED_FACE`; empty if it is not defined.
    pub entity: String,
    /// The usage that names it at the end of [`via`](Self::via).
    pub usage: u64,
    /// One shortest route from the definition to the aspect that usage
    /// names; empty if it names the definition itself.
    pub via: Vec<Step>,
    /// The kinds of step each route to the item takes, each distinct set
    /// once, in a fixed order: the empty set if a usage names the definition
    /// itself. A value stated on a composite applies to the group, and an
    /// axis's derived-from faces are not what it tolerances, so a caller
    /// keeps these apart.
    pub route_kinds: Vec<StepKinds>,
}

/// Where a route stopped short of an item.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Unresolved {
    /// The aspect where it stopped.
    pub aspect: u64,
    /// Its upper-cased entity types, joined with `+`; empty if it is not
    /// defined.
    pub entity: String,
    /// One shortest route to it.
    pub via: Vec<Step>,
    /// Why it stopped.
    pub reason: UnresolvedReason,
}

/// Why a route stopped; see [`Unresolved`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[non_exhaustive]
pub enum UnresolvedReason {
    /// No followed usage names the aspect, and no rule leads on from it.
    NoUsage,
    /// A `product_definition_shape`: the whole shape of a product, not a
    /// part of it.
    WholeShape,
    /// A plain `shape_aspect_relationship` from here, or into a datum from
    /// something that is neither a datum feature nor a datum target, which
    /// no rule gives a meaning to; only a description string says what it
    /// is.
    NotFollowed {
        /// The relationship.
        relationship: u64,
        /// The aspect on its other side.
        related: u64,
    },
    /// A relationship of another subtype, or a complex one, from here, or
    /// into a datum.
    UnsupportedRelationship {
        /// The relationship.
        relationship: u64,
        /// Its upper-cased entity types, joined with `+`.
        entity: String,
    },
    /// A usage of kind [`Other`](ItemUsageKind::Other) names the aspect.
    UnsupportedUsage {
        /// The usage.
        usage: u64,
    },
    /// A definition stepq does not resolve: a dimension, or a relationship,
    /// as a tolerance's target.
    UnsupportedDefinition,
    /// A `component_path_shape_aspect`, which names another product's
    /// aspect through an assembly occurrence.
    ComponentPath,
    /// A usage names an item that is not defined.
    MissingItem {
        /// The usage.
        usage: u64,
        /// The item.
        item: u64,
    },
    /// The aspect is not defined.
    Missing,
    /// The relationships lead back to an aspect already on a route to here.
    Cycle,
    /// The aspect lies 64 relationships from the definition by the
    /// shortest route, and its steps on are not followed.
    TooDeep,
}

impl fmt::Display for UnresolvedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoUsage => f.write_str("nothing names its items"),
            Self::WholeShape => f.write_str("the whole shape of a product"),
            Self::NotFollowed {
                relationship,
                related,
            } => write!(
                f,
                "relationship #{relationship} to #{related} has no stated meaning"
            ),
            Self::UnsupportedRelationship {
                relationship,
                entity,
            } => write!(f, "relationship #{relationship} ({entity}) is not read"),
            Self::UnsupportedUsage { usage } => write!(f, "usage #{usage} is not read"),
            Self::UnsupportedDefinition => f.write_str("a dimension or relationship as a target"),
            Self::ComponentPath => f.write_str("an aspect of another product, by assembly path"),
            Self::MissingItem { usage, item } => {
                write!(f, "usage #{usage} names #{item}, which is not defined")
            }
            Self::Missing => f.write_str("not defined"),
            Self::Cycle => f.write_str("the relationships lead back to it"),
            Self::TooDeep => f.write_str("64 relationships away, not followed further"),
        }
    }
}

/// A definition whose [`AspectItems::resolve`] lists an item, from
/// [`AspectItems::naming`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Naming {
    /// The definition's `#id`.
    pub definition: u64,
    /// The kinds of step each of its routes to the item takes, as in
    /// [`ResolvedItem::route_kinds`].
    pub route_kinds: Vec<StepKinds>,
}

impl Naming {
    /// True if a usage names the item for the definition itself.
    pub fn is_direct(&self) -> bool {
        self.route_kinds.contains(&StepKinds::NONE)
    }
}

/// What a relationship is to the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelationshipKind {
    /// A simple `shape_aspect_relationship` of no subtype.
    Plain,
    /// A simple `shape_aspect_deriving_relationship`.
    Deriving,
    /// A simple `feature_for_datum_target_relationship`.
    TargetFeature,
    /// A dimension: a `dimensional_location`, of any subtype.
    Location,
    /// Any other subtype, or a complex instance.
    Other,
}

#[derive(Debug, Clone)]
struct Relationship {
    instance: u64,
    kind: RelationshipKind,
    relating: u64,
    /// `None` if it is not a reference: the relationship is reported.
    related: Option<u64>,
}

/// What an aspect is, for the rules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Class(u8);

impl Class {
    const COMPOSITE: u8 = 1;
    const DERIVED: u8 = 2;
    const DATUM_FEATURE: u8 = 4;
    const DATUM_TARGET: u8 = 8;
    const DATUM: u8 = 16;
    const BETWEEN: u8 = 32;

    fn of(names: &[String]) -> Self {
        let has = |set: &[&str]| names.iter().any(|name| set.contains(&name.as_str()));
        let mut class = 0;
        for (set, bit) in [
            (COMPOSITES, Self::COMPOSITE),
            (DERIVED, Self::DERIVED),
            (DATUM_FEATURES, Self::DATUM_FEATURE),
            (DATUM_TARGETS, Self::DATUM_TARGET),
            (DATUMS, Self::DATUM),
            (&["BETWEEN_SHAPE_ASPECT"], Self::BETWEEN),
        ] {
            if has(set) {
                class |= bit;
            }
        }
        Self(class)
    }

    fn is(self, bit: u8) -> bool {
        self.0 & bit != 0
    }
}

/// The representation items shape aspects stand for, read once per file
/// from every `item_identified_representation_usage` and every
/// `shape_aspect_relationship`.
///
/// [`resolve`](Self::resolve) follows only links whose meaning the schema
/// fixes, from a definition (a shape aspect, datum, or dimension with its
/// own usage) to the items the file names for it:
///
/// * a simple `geometric_item_specific_usage` or plain
///   `item_identified_representation_usage`, to its `identified_item` (one
///   reference, or the members of a typed `set_representation_item` or
///   `list_representation_item`);
/// * [`Component`](StepKind::Component), [`DerivedFrom`](StepKind::DerivedFrom),
///   [`DatumFeature`](StepKind::DatumFeature),
///   [`DatumTarget`](StepKind::DatumTarget) and
///   [`TargetFeature`](StepKind::TargetFeature) steps between aspects.
///
/// Presentation usages are listed in [`usages`](Self::usages) and never
/// followed. Everything else that could lead to items is reported in
/// [`Resolution::unresolved`] with a reason: nothing is guessed, and
/// nothing is skipped. Each aspect is expanded once per resolution, so a
/// resolution takes time linear in the file however its aspects share
/// components. The index owns its data; `resolve` takes the graph again to
/// name entity types.
///
/// stepq does not say which face of a group a value measures, does not
/// check that items lie in `used_representation`, and does not follow
/// aspect shapes given through `property_definition` (the same pattern
/// carries property values such as a centroid).
#[derive(Debug, Clone)]
pub struct AspectItems {
    usages: Vec<ItemUsage>,
    /// Each usage's items, each once, for resolving.
    distinct: Vec<Vec<u64>>,
    /// Followed usages, by definition.
    by_definition: HashMap<u64, Vec<usize>>,
    /// Followed usages, by item.
    by_item: HashMap<u64, Vec<usize>>,
    /// Usages of kind Other, by definition.
    unsupported: HashMap<u64, Vec<u64>>,
    relationships: Vec<Relationship>,
    /// Relationships by relating aspect.
    from: HashMap<u64, Vec<usize>>,
    /// Relationships by related aspect.
    to: HashMap<u64, Vec<usize>>,
    /// What each aspect a usage or relationship names is.
    classes: HashMap<u64, Class>,
    /// The aspects `resolve` stops at without reading their usages: not
    /// defined, a whole shape, an assembly path, or a definition it does
    /// not resolve.
    stops: HashSet<u64>,
}

fn names(exchange: &Exchange<'_>, instance: &Instance) -> Vec<String> {
    exchange
        .records(instance)
        .map(|record| record::upper(record.name()))
        .collect()
}

fn entity(exchange: &Exchange<'_>, id: u64) -> String {
    exchange
        .get(id)
        .map(|instance| record::entity(exchange, instance))
        .unwrap_or_default()
}

/// The members of an `identified_item`: one reference, or a typed set or
/// list of them.
fn identified_items(param: Option<Param<'_>>) -> Option<Vec<u64>> {
    match param? {
        Param::Reference(item) => Some(vec![item]),
        Param::Typed(typed)
            if typed.is("SET_REPRESENTATION_ITEM") || typed.is("LIST_REPRESENTATION_ITEM") =>
        {
            let items = typed
                .param(0)?
                .list()?
                .map(|member| member.reference())
                .collect::<Option<Vec<u64>>>()?;
            (!items.is_empty()).then_some(items)
        }
        _ => None,
    }
}

impl AspectItems {
    /// Reads every usage and relationship of the file behind `graph`, built
    /// with [`Graph::new`] or [`Graph::build`]. Never fails: a usage or
    /// relationship that cannot be read is reported where a resolution
    /// meets it.
    pub fn new(graph: &Graph<'_>) -> Self {
        let exchange = graph.exchange();
        let mut index = Self {
            usages: Vec::new(),
            distinct: Vec::new(),
            by_definition: HashMap::new(),
            by_item: HashMap::new(),
            unsupported: HashMap::new(),
            relationships: Vec::new(),
            from: HashMap::new(),
            to: HashMap::new(),
            classes: HashMap::new(),
            stops: HashSet::new(),
        };
        for instance in exchange.instances() {
            let names = names(exchange, instance);
            if names.iter().any(|name| USAGES.contains(&name.as_str())) {
                index.usage(exchange, instance, &names);
            } else if names
                .iter()
                .any(|name| RELATIONSHIPS.contains(&name.as_str()))
            {
                index.relationship(exchange, instance, &names);
            }
        }
        let mut aspects: HashSet<u64> = index.by_definition.keys().copied().collect();
        aspects.extend(index.unsupported.keys());
        for relationship in &index.relationships {
            aspects.insert(relationship.relating);
            aspects.extend(relationship.related);
        }
        for aspect in aspects {
            let Some(instance) = exchange.get(aspect) else {
                index.stops.insert(aspect);
                continue;
            };
            let names = names(exchange, instance);
            if stop(&names).is_some() {
                index.stops.insert(aspect);
            }
            index.classes.insert(aspect, Class::of(&names));
        }
        index
            .by_item
            .retain(|&item, _| exchange.get(item).is_some());
        index
    }

    fn usage(&mut self, exchange: &Exchange<'_>, instance: &Instance, names: &[String]) {
        let record: Option<Record<'_>> = if instance.is_complex() {
            record::partial(exchange, instance, "ITEM_IDENTIFIED_REPRESENTATION_USAGE")
        } else {
            exchange.records(instance).next()
        };
        let Some(record) = record else {
            return;
        };
        let Some(definition) = record::reference(record.param(2).as_ref()) else {
            return;
        };
        let items = identified_items(record.param(4));
        let simple = |name: &str| !instance.is_complex() && names == [name];
        let kind = if names
            .iter()
            .any(|name| PRESENTATION.contains(&name.as_str()))
        {
            ItemUsageKind::Presentation
        } else if items.is_none() {
            ItemUsageKind::Other
        } else if simple("GEOMETRIC_ITEM_SPECIFIC_USAGE") {
            ItemUsageKind::GeometricItemSpecific
        } else if simple("ITEM_IDENTIFIED_REPRESENTATION_USAGE") {
            ItemUsageKind::ItemIdentified
        } else {
            ItemUsageKind::Other
        };
        let index = self.usages.len();
        let mut seen = HashSet::new();
        let distinct: Vec<u64> = items
            .iter()
            .flatten()
            .copied()
            .filter(|&item| seen.insert(item))
            .collect();
        match kind {
            ItemUsageKind::GeometricItemSpecific | ItemUsageKind::ItemIdentified => {
                self.by_definition
                    .entry(definition)
                    .or_default()
                    .push(index);
                for &item in &distinct {
                    self.by_item.entry(item).or_default().push(index);
                }
            }
            ItemUsageKind::Other => {
                self.unsupported
                    .entry(definition)
                    .or_default()
                    .push(instance.id);
            }
            ItemUsageKind::Presentation => {}
        }
        self.distinct.push(distinct);
        self.usages.push(ItemUsage {
            instance: instance.id,
            kind,
            entity: names.join("+"),
            name: text(record.param(0)),
            description: text(record.param(1)),
            definition,
            used_representation: record::reference(record.param(3).as_ref()),
            items: items.unwrap_or_default(),
        });
    }

    fn relationship(&mut self, exchange: &Exchange<'_>, instance: &Instance, names: &[String]) {
        let (record, offset) = if instance.is_complex() {
            (
                record::partial(exchange, instance, "SHAPE_ASPECT_RELATIONSHIP"),
                0,
            )
        } else {
            let offset = RELATIONSHIP_OFFSETS
                .iter()
                .find(|(name, _)| names == [*name])
                .map_or(0, |(_, offset)| *offset);
            (exchange.records(instance).next(), offset)
        };
        let Some(record) = record else {
            return;
        };
        // A relationship whose relating aspect cannot be read belongs to no
        // aspect; one whose related aspect cannot be read is reported from
        // its relating one.
        let Some(relating) = record::reference(record.param(offset + 2).as_ref()) else {
            return;
        };
        let related = record::reference(record.param(offset + 3).as_ref());
        let simple = |name: &str| !instance.is_complex() && names == [name];
        let kind = if related.is_none() {
            RelationshipKind::Other
        } else if names.iter().any(|name| LOCATIONS.contains(&name.as_str())) {
            RelationshipKind::Location
        } else if simple("SHAPE_ASPECT_RELATIONSHIP") {
            RelationshipKind::Plain
        } else if simple("SHAPE_ASPECT_DERIVING_RELATIONSHIP") {
            RelationshipKind::Deriving
        } else if simple("FEATURE_FOR_DATUM_TARGET_RELATIONSHIP") {
            RelationshipKind::TargetFeature
        } else {
            RelationshipKind::Other
        };
        let index = self.relationships.len();
        self.relationships.push(Relationship {
            instance: instance.id,
            kind,
            relating,
            related,
        });
        self.from.entry(relating).or_default().push(index);
        if let Some(related) = related {
            self.to.entry(related).or_default().push(index);
        }
    }

    /// Every `item_identified_representation_usage` whose `definition` is a
    /// reference, in file order, presentation links included.
    pub fn usages(&self) -> &[ItemUsage] {
        &self.usages
    }

    fn class(&self, aspect: u64) -> Class {
        self.classes.get(&aspect).copied().unwrap_or_default()
    }

    /// The steps from `#aspect`, the reasons it stops there, and the
    /// relationships from it that are not read.
    fn steps(&self, aspect: u64, class: Class) -> (Vec<Step>, Vec<UnresolvedReason>, Vec<u64>) {
        let mut steps = Vec::new();
        let mut refused = Vec::new();
        let mut others = Vec::new();
        for &index in self.from.get(&aspect).into_iter().flatten() {
            let relationship = &self.relationships[index];
            let Some(related) = relationship.related else {
                others.push(relationship.instance);
                continue;
            };
            let basis = (class.is(Class::DATUM_FEATURE) || class.is(Class::DATUM_TARGET))
                && self.class(related).is(Class::DATUM);
            match relationship.kind {
                // A datum feature's or target's basis; a between aspect's
                // start and end features, which bound its region.
                RelationshipKind::Plain if basis || class.is(Class::BETWEEN) => {}
                RelationshipKind::Plain if class.is(Class::COMPOSITE) => steps.push(Step {
                    relationship: relationship.instance,
                    kind: StepKind::Component,
                    aspect: related,
                }),
                RelationshipKind::Plain => refused.push(UnresolvedReason::NotFollowed {
                    relationship: relationship.instance,
                    related,
                }),
                RelationshipKind::Deriving if class.is(Class::DERIVED) => steps.push(Step {
                    relationship: relationship.instance,
                    kind: StepKind::DerivedFrom,
                    aspect: related,
                }),
                // Read from the target's side, and dimensions.
                RelationshipKind::TargetFeature | RelationshipKind::Location => {}
                RelationshipKind::Deriving | RelationshipKind::Other => {
                    others.push(relationship.instance);
                }
            }
        }
        for &index in self.to.get(&aspect).into_iter().flatten() {
            let relationship = &self.relationships[index];
            let relating = relationship.relating;
            let into_datum = class.is(Class::DATUM);
            match relationship.kind {
                RelationshipKind::Plain if into_datum => {
                    let from = self.class(relating);
                    let kind = if from.is(Class::DATUM_FEATURE) {
                        Some(StepKind::DatumFeature)
                    } else if from.is(Class::DATUM_TARGET) {
                        Some(StepKind::DatumTarget)
                    } else {
                        None
                    };
                    match kind {
                        Some(kind) => steps.push(Step {
                            relationship: relationship.instance,
                            kind,
                            aspect: relating,
                        }),
                        // A common datum's component: read from its side.
                        None if from.is(Class::COMPOSITE) => {}
                        None => refused.push(UnresolvedReason::NotFollowed {
                            relationship: relationship.instance,
                            related: relating,
                        }),
                    }
                }
                RelationshipKind::TargetFeature if class.is(Class::DATUM_TARGET) => {
                    steps.push(Step {
                        relationship: relationship.instance,
                        kind: StepKind::TargetFeature,
                        aspect: relating,
                    });
                }
                RelationshipKind::Other if into_datum || class.is(Class::DATUM_TARGET) => {
                    others.push(relationship.instance);
                }
                _ => {}
            }
        }
        (steps, refused, others)
    }

    /// What `#definition` (a shape aspect, datum, or dimension with its own
    /// usage) stands for: each item it reaches, once, with one shortest
    /// route and the kinds of step every route takes; and each point where a
    /// route stopped, with the reason. `graph` must be the graph the index
    /// was read from.
    pub fn resolve(&self, graph: &Graph<'_>, definition: u64) -> Resolution {
        let exchange = graph.exchange();
        let walked = self.walk(exchange, definition);
        let routes = routes(definition, &walked.edges);
        let mut out = Resolution {
            definition,
            items: Vec::new(),
            unresolved: Vec::new(),
        };
        let mut items: HashMap<u64, (usize, Routes)> = HashMap::new();
        for &aspect in &walked.order {
            let aspect_routes = routes.get(&aspect).copied().unwrap_or_default();
            for &index in self.by_definition.get(&aspect).into_iter().flatten() {
                let usage = &self.usages[index];
                for &item in &self.distinct[index] {
                    if exchange.get(item).is_none() {
                        continue;
                    }
                    if let Some((_, found)) = items.get_mut(&item) {
                        found.0 |= aspect_routes.0;
                        continue;
                    }
                    items.insert(item, (out.items.len(), aspect_routes));
                    out.items.push(ResolvedItem {
                        item,
                        entity: entity(exchange, item),
                        usage: usage.instance,
                        via: walked.via(aspect),
                        route_kinds: Vec::new(),
                    });
                }
            }
        }
        for (index, found) in items.into_values() {
            out.items[index].route_kinds = found.sets();
        }
        out.unresolved = walked
            .stops
            .iter()
            .map(|(aspect, reason)| Unresolved {
                aspect: *aspect,
                entity: entity(exchange, *aspect),
                via: walked.via(*aspect),
                reason: reason.clone(),
            })
            .collect();
        // Cycles: a step back to an aspect on the current depth-first path.
        for (from, step) in back_edges(definition, &walked.edges) {
            let mut via = walked.via(from);
            via.push(step);
            out.unresolved.push(Unresolved {
                aspect: step.aspect,
                entity: entity(exchange, step.aspect),
                via,
                reason: UnresolvedReason::Cycle,
            });
        }
        out
    }

    /// Walks from `#definition` breadth first, each aspect once: the
    /// aspects whose items it lists, how each was first reached, the steps
    /// on from each, and where it stopped.
    fn walk(&self, exchange: &Exchange<'_>, definition: u64) -> Walked {
        let mut walked = Walked {
            order: Vec::new(),
            reached: HashMap::from([(definition, (0, None))]),
            edges: HashMap::new(),
            stops: Vec::new(),
        };
        let mut queue = VecDeque::from([definition]);
        while let Some(aspect) = queue.pop_front() {
            let depth = walked.reached[&aspect].0;
            // An aspect it stops at lists none of its items, as `naming`
            // does not.
            let Some(instance) = exchange.get(aspect) else {
                walked.stops.push((aspect, UnresolvedReason::Missing));
                continue;
            };
            let names = names(exchange, instance);
            if let Some(reason) = stop(&names) {
                walked.stops.push((aspect, reason));
                continue;
            }
            walked.order.push(aspect);
            let steps = self.facts(exchange, aspect, &names, &mut walked.stops);
            if steps.is_empty() {
                continue;
            }
            if depth >= MAX_DEPTH {
                walked.stops.push((aspect, UnresolvedReason::TooDeep));
                continue;
            }
            for step in &steps {
                if let Entry::Vacant(entry) = walked.reached.entry(step.aspect) {
                    entry.insert((depth + 1, Some((aspect, *step))));
                    queue.push_back(step.aspect);
                }
            }
            walked.edges.insert(aspect, steps);
        }
        walked
    }

    /// What `#aspect` states: its stops, pushed to `stops`, and its steps
    /// on, returned.
    fn facts(
        &self,
        exchange: &Exchange<'_>,
        aspect: u64,
        names: &[String],
        stops: &mut Vec<(u64, UnresolvedReason)>,
    ) -> Vec<Step> {
        let mut stated = false;
        for &index in self.by_definition.get(&aspect).into_iter().flatten() {
            stated = true;
            let usage = &self.usages[index];
            for &item in &self.distinct[index] {
                if exchange.get(item).is_none() {
                    stops.push((
                        aspect,
                        UnresolvedReason::MissingItem {
                            usage: usage.instance,
                            item,
                        },
                    ));
                }
            }
        }
        for &usage in self.unsupported.get(&aspect).into_iter().flatten() {
            stated = true;
            stops.push((aspect, UnresolvedReason::UnsupportedUsage { usage }));
        }
        let (onward, reasons, others) = self.steps(aspect, Class::of(names));
        stated |= !reasons.is_empty() || !others.is_empty();
        stops.extend(reasons.into_iter().map(|reason| (aspect, reason)));
        for relationship in others {
            stops.push((
                aspect,
                UnresolvedReason::UnsupportedRelationship {
                    relationship,
                    entity: entity(exchange, relationship),
                },
            ));
        }
        if !stated && onward.is_empty() {
            stops.push((aspect, UnresolvedReason::NoUsage));
        }
        onward
    }

    /// The definitions whose [`resolve`](Self::resolve) lists `#item`, in
    /// ascending order of `#id`, each with the kinds of step its routes
    /// take: walked back from the usages that name the item, each aspect
    /// once. Empty if no followed usage names it. Both limit routes to 64
    /// relationships, `resolve` counting from the definition and this from
    /// the item, so for a route longer than that the kinds can differ;
    /// which definitions are listed does not.
    pub fn naming(&self, item: u64) -> Vec<Naming> {
        let mut routes: HashMap<u64, Routes> = HashMap::new();
        let mut depth: HashMap<u64, usize> = HashMap::new();
        let mut queue: VecDeque<u64> = VecDeque::new();
        for &index in self.by_item.get(&item).into_iter().flatten() {
            let aspect = self.usages[index].definition;
            if !self.stops.contains(&aspect) && routes.insert(aspect, Routes::DIRECT).is_none() {
                depth.insert(aspect, 0);
                queue.push_back(aspect);
            }
        }
        // Breadth first, back from the item, for the depth limit: each
        // aspect's steps into it, once.
        let mut into: HashMap<u64, Vec<(u64, StepKind)>> = HashMap::new();
        while let Some(aspect) = queue.pop_front() {
            let here = depth[&aspect];
            if here >= MAX_DEPTH {
                continue;
            }
            let steps: Vec<(u64, StepKind)> = self
                .steps_into(aspect)
                .into_iter()
                .filter(|(from, _)| !self.stops.contains(from))
                .collect();
            for &(from, _) in &steps {
                if let Entry::Vacant(entry) = depth.entry(from) {
                    entry.insert(here + 1);
                    queue.push_back(from);
                }
            }
            into.insert(aspect, steps);
        }
        // Then the kinds of every route, to a fixed point, as `resolve` does
        // forward.
        let mut work: Vec<u64> = routes.keys().copied().collect();
        while let Some(aspect) = work.pop() {
            let here = routes.get(&aspect).copied().unwrap_or_default();
            for &(from, kind) in into.get(&aspect).into_iter().flatten() {
                let there = routes.entry(from).or_default();
                let grown = Routes(there.0 | here.then(kind).0);
                if grown != *there {
                    *there = grown;
                    work.push(from);
                }
            }
        }
        let mut naming: Vec<Naming> = routes
            .into_iter()
            .map(|(definition, routes)| Naming {
                definition,
                route_kinds: routes.sets(),
            })
            .collect();
        naming.sort_by_key(|naming| naming.definition);
        naming
    }

    /// The aspects with a step to `#aspect`, and the kind of each.
    fn steps_into(&self, aspect: u64) -> Vec<(u64, StepKind)> {
        let class = self.class(aspect);
        let mut found = Vec::new();
        for &index in self.to.get(&aspect).into_iter().flatten() {
            let relationship = &self.relationships[index];
            let from = self.class(relationship.relating);
            let basis = (from.is(Class::DATUM_FEATURE) || from.is(Class::DATUM_TARGET))
                && class.is(Class::DATUM);
            match relationship.kind {
                RelationshipKind::Plain
                    if from.is(Class::COMPOSITE) && !basis && !from.is(Class::BETWEEN) =>
                {
                    found.push((relationship.relating, StepKind::Component));
                }
                RelationshipKind::Deriving if from.is(Class::DERIVED) => {
                    found.push((relationship.relating, StepKind::DerivedFrom));
                }
                _ => {}
            }
        }
        for &index in self.from.get(&aspect).into_iter().flatten() {
            let relationship = &self.relationships[index];
            let Some(related) = relationship.related else {
                continue;
            };
            let to = self.class(related);
            match relationship.kind {
                RelationshipKind::Plain if to.is(Class::DATUM) => {
                    if class.is(Class::DATUM_FEATURE) {
                        found.push((related, StepKind::DatumFeature));
                    } else if class.is(Class::DATUM_TARGET) {
                        found.push((related, StepKind::DatumTarget));
                    }
                }
                RelationshipKind::TargetFeature if to.is(Class::DATUM_TARGET) => {
                    found.push((related, StepKind::TargetFeature));
                }
                _ => {}
            }
        }
        found
    }
}

/// What [`AspectItems::walk`] found.
struct Walked {
    /// The aspects whose items the resolution lists, nearest first.
    order: Vec<u64>,
    /// Each aspect reached: its depth, and the aspect and step that first
    /// reached it.
    reached: HashMap<u64, (usize, Option<(u64, Step)>)>,
    /// The steps on from each aspect expanded.
    edges: HashMap<u64, Vec<Step>>,
    /// Where routes stopped, and why.
    stops: Vec<(u64, UnresolvedReason)>,
}

impl Walked {
    /// One shortest route to `#aspect`.
    fn via(&self, aspect: u64) -> Vec<Step> {
        let mut route = Vec::new();
        let mut at = aspect;
        while let Some(&(_, Some((from, step)))) = self.reached.get(&at) {
            route.push(step);
            at = from;
        }
        route.reverse();
        route
    }
}

/// The kinds of every route from `#root` to each aspect, to a fixed point:
/// each aspect's set of subsets only grows, and has 32 bits.
fn routes(root: u64, edges: &HashMap<u64, Vec<Step>>) -> HashMap<u64, Routes> {
    let mut routes: HashMap<u64, Routes> = HashMap::from([(root, Routes::DIRECT)]);
    let mut work: Vec<u64> = vec![root];
    while let Some(aspect) = work.pop() {
        let here = routes[&aspect];
        for step in edges.get(&aspect).into_iter().flatten() {
            let there = routes.entry(step.aspect).or_default();
            let grown = Routes(there.0 | here.then(step.kind).0);
            if grown != *there {
                *there = grown;
                work.push(step.aspect);
            }
        }
    }
    routes
}

/// Why `resolve` stops at an aspect of these entity types without reading
/// its usages, if it does.
fn stop(names: &[String]) -> Option<UnresolvedReason> {
    let has = |set: &[&str]| names.iter().any(|name| set.contains(&name.as_str()));
    if has(&["PRODUCT_DEFINITION_SHAPE"]) {
        Some(UnresolvedReason::WholeShape)
    } else if has(&["COMPONENT_PATH_SHAPE_ASPECT"]) {
        Some(UnresolvedReason::ComponentPath)
    } else if (has(SIZES) || has(RELATIONSHIPS)) && !has(DATUM_FEATURES) {
        Some(UnresolvedReason::UnsupportedDefinition)
    } else {
        None
    }
}

/// The steps of `edges`, each with the aspect it leaves, that lead back to
/// an aspect on the depth-first path from `root`: one per cycle entry.
fn back_edges(root: u64, edges: &HashMap<u64, Vec<Step>>) -> Vec<(u64, Step)> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Colour {
        Grey,
        Black,
    }
    let mut colour: HashMap<u64, Colour> = HashMap::from([(root, Colour::Grey)]);
    let mut stack: Vec<(u64, usize)> = vec![(root, 0)];
    let mut found = Vec::new();
    while let Some(&mut (aspect, ref mut next)) = stack.last_mut() {
        let steps = edges.get(&aspect).map_or(&[][..], Vec::as_slice);
        if let Some(step) = steps.get(*next) {
            *next += 1;
            match colour.get(&step.aspect) {
                Some(Colour::Grey) => found.push((aspect, *step)),
                Some(Colour::Black) => {}
                None => {
                    colour.insert(step.aspect, Colour::Grey);
                    stack.push((step.aspect, 0));
                }
            }
        } else {
            colour.insert(aspect, Colour::Black);
            stack.pop();
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use crate::p21::parse;

    fn read(data: &str) -> (&'static Graph<'static>, AspectItems) {
        let src = format!(
            "ISO-10303-21;HEADER;FILE_SCHEMA(('AP242'));ENDSEC;DATA;\n{data}\nENDSEC;END-ISO-10303-21;"
        );
        let src: &'static [u8] = Box::leak(src.into_bytes().into_boxed_slice());
        // build, not new: some cases refer to instances that are not defined.
        let graph: &'static Graph<'static> = Box::leak(Box::new(Graph::build(parse(src).unwrap())));
        let index = AspectItems::new(graph);
        (graph, index)
    }

    /// Faces #1 to #9, a representation and a shape.
    const BASE: &str = "#1=ADVANCED_FACE('',(),#90,.T.);#2=ADVANCED_FACE('',(),#90,.T.);\
#3=ADVANCED_FACE('',(),#90,.T.);#4=ADVANCED_FACE('',(),#90,.T.);#5=TRIMMED_CURVE('',#91,(),(),.T.,.UNSPECIFIED.);\
#6=CARTESIAN_POINT('',(0.,0.,0.));#20=SHAPE_REPRESENTATION('',(#1),#92);#21=PRODUCT_DEFINITION_SHAPE('','',#93);";

    fn ids(resolution: &Resolution) -> Vec<u64> {
        resolution.items.iter().map(|item| item.item).collect()
    }

    fn reasons(resolution: &Resolution) -> Vec<(u64, UnresolvedReason)> {
        resolution
            .unresolved
            .iter()
            .map(|u| (u.aspect, u.reason.clone()))
            .collect()
    }

    fn gisu(id: u64, aspect: u64, item: u64) -> String {
        format!("#{id}=GEOMETRIC_ITEM_SPECIFIC_USAGE('','',#{aspect},#20,#{item});")
    }

    #[test]
    fn usages_name_items_directly() {
        let data = format!(
            "{BASE}#30=SHAPE_ASPECT('','',#21,.T.);{}\
             #41=ITEM_IDENTIFIED_REPRESENTATION_USAGE('','',#31,#20,SET_REPRESENTATION_ITEM((#2,#3)));\
             #42=ITEM_IDENTIFIED_REPRESENTATION_USAGE('','',#32,#20,LIST_REPRESENTATION_ITEM((#4,#3)));\
             #43=ITEM_IDENTIFIED_REPRESENTATION_USAGE('','',#33,#20,(#1,#2));\
             #44=ITEM_IDENTIFIED_REPRESENTATION_USAGE('','',#33,#20,'x');\
             #31=SHAPE_ASPECT('','',#21,.T.);#32=SHAPE_ASPECT('','',#21,.T.);#33=SHAPE_ASPECT('','',#21,.T.);",
            gisu(40, 30, 1)
        );
        let (graph, index) = read(&data);
        let one = index.resolve(graph, 30);
        assert_eq!(ids(&one), [1]);
        assert!(one.is_complete());
        assert_eq!(one.items[0].entity, "ADVANCED_FACE");
        assert_eq!(one.items[0].usage, 40);
        assert_eq!(one.items[0].via, []);
        assert_eq!(one.items[0].route_kinds, [StepKinds::NONE]);
        assert_eq!(ids(&index.resolve(graph, 31)), [2, 3]);
        assert_eq!(
            ids(&index.resolve(graph, 32)),
            [4, 3],
            "a list keeps its order"
        );
        let kinds: Vec<ItemUsageKind> = index.usages().iter().map(|u| u.kind).collect();
        assert_eq!(
            kinds,
            [
                ItemUsageKind::GeometricItemSpecific,
                ItemUsageKind::ItemIdentified,
                ItemUsageKind::ItemIdentified,
                ItemUsageKind::Other,
                ItemUsageKind::Other,
            ]
        );
        assert_eq!(
            reasons(&index.resolve(graph, 33)),
            [
                (33, UnresolvedReason::UnsupportedUsage { usage: 43 }),
                (33, UnresolvedReason::UnsupportedUsage { usage: 44 }),
            ]
        );
    }

    /// A group of a composite and an aspect, with an all-around: two
    /// component steps, each item once, by its shortest route.
    #[test]
    fn composites_and_their_components() {
        let data = format!(
            "{BASE}#30=COMPOSITE_GROUP_SHAPE_ASPECT('','',#21,.T.);#31=COMPOSITE_SHAPE_ASPECT('','',#21,.T.);\
             #32=SHAPE_ASPECT('','',#21,.T.);#33=SHAPE_ASPECT('','',#21,.T.);#34=ALL_AROUND_SHAPE_ASPECT('','',#21,.T.);\
             #50=SHAPE_ASPECT_RELATIONSHIP('','',#30,#31);#51=SHAPE_ASPECT_RELATIONSHIP('','',#31,#32);\
             #52=SHAPE_ASPECT_RELATIONSHIP('','',#31,#33);#53=SHAPE_ASPECT_RELATIONSHIP('','',#30,#33);\
             #54=SHAPE_ASPECT_RELATIONSHIP('','',#34,#32);{}{}{}",
            gisu(40, 32, 1),
            gisu(41, 33, 2),
            gisu(42, 30, 3)
        );
        let (graph, index) = read(&data);
        let group = index.resolve(graph, 30);
        assert!(group.is_complete(), "{:?}", group.unresolved);
        assert_eq!(ids(&group), [3, 2, 1], "nearest first, each once");
        let component = StepKinds::NONE.with(StepKind::Component);
        assert_eq!(group.items[0].route_kinds, [StepKinds::NONE]);
        assert_eq!(
            group.items[1].route_kinds,
            [component],
            "two routes, one kind"
        );
        assert_eq!(group.items[1].via.len(), 1, "the shorter route");
        assert_eq!(group.items[2].via.len(), 2);
        assert_eq!(group.item_ids(StepKinds::NONE), [3]);
        assert_eq!(group.item_ids(component), [3, 2, 1]);
        assert_eq!(ids(&index.resolve(graph, 34)), [1]);
        let naming = index.naming(2);
        let definitions: Vec<u64> = naming.iter().map(|n| n.definition).collect();
        assert_eq!(definitions, [30, 31, 33]);
        assert!(naming[2].is_direct() && !naming[0].is_direct());
    }

    /// A datum feature that is also a composite: its relationship to its
    /// datum is its basis, not a component, so the datum reaches its faces
    /// by a datum-feature step and nothing cycles.
    #[test]
    fn datum_features_targets_and_derived_aspects() {
        let data = format!(
            "{BASE}#30=DATUM('',$,#21,.F.,'A');\
             #31=(COMPOSITE_SHAPE_ASPECT()DATUM_FEATURE()SHAPE_ASPECT('','',#21,.T.));\
             #32=SHAPE_ASPECT('','',#21,.T.);#50=SHAPE_ASPECT_RELATIONSHIP('','',#31,#30);\
             #51=SHAPE_ASPECT_RELATIONSHIP('','',#31,#32);{}\
             #33=DATUM('',$,#21,.F.,'B');#34=PLACED_DATUM_TARGET_FEATURE('','point',#21,.T.,'1');\
             #35=SHAPE_ASPECT('','',#21,.T.);#52=SHAPE_ASPECT_RELATIONSHIP('','',#34,#33);\
             #53=FEATURE_FOR_DATUM_TARGET_RELATIONSHIP('','',#35,#34);{}{}\
             #36=CENTRE_OF_SYMMETRY('','',#21,.T.);#54=SHAPE_ASPECT_DERIVING_RELATIONSHIP('','',#36,#32);{}",
            gisu(40, 32, 1),
            gisu(41, 34, 6),
            gisu(42, 35, 2),
            gisu(43, 36, 5)
        );
        let (graph, index) = read(&data);
        let a = index.resolve(graph, 30);
        assert!(a.is_complete(), "{:?}", a.unresolved);
        assert_eq!(ids(&a), [1]);
        assert_eq!(
            a.items[0].via.iter().map(|s| s.kind).collect::<Vec<_>>(),
            [StepKind::DatumFeature, StepKind::Component]
        );
        let b = index.resolve(graph, 33);
        assert_eq!(ids(&b), [6, 2]);
        assert_eq!(b.items[1].via[1].kind, StepKind::TargetFeature);
        let axis = index.resolve(graph, 36);
        assert_eq!(ids(&axis), [5, 1]);
        assert_eq!(
            axis.items[1].route_kinds,
            [StepKinds::NONE.with(StepKind::DerivedFrom)]
        );
        assert_eq!(
            axis.item_ids(StepKinds::NONE),
            [5],
            "the axis, not the faces"
        );
    }

    #[test]
    fn stops_are_reported() {
        let data = format!(
            "{BASE}#30=DATUM_FEATURE('','',#21,.T.);#31=SHAPE_ASPECT('','',#21,.T.);\
             #50=SHAPE_ASPECT_RELATIONSHIP('datum feature','',#30,#31);{}\
             #32=SHAPE_ASPECT('','',#21,.T.);#33=COMPONENT_PATH_SHAPE_ASPECT('','',#21,.T.,#94,#95);\
             #34=SHAPE_ASPECT('','',#21,.T.);\
             #44=CHAIN_BASED_ITEM_IDENTIFIED_REPRESENTATION_USAGE('','',#34,#20,#1,(#20),$);\
             #45=DRAUGHTING_MODEL_ITEM_ASSOCIATION('','',#32,#96,#97);\
             #46=DRAUGHTING_MODEL_ITEM_ASSOCIATION_WITH_EXTERNAL_IMAGE_PLACEMENT('','',#32,#96,#97,#98);\
             #35=COMPOSITE_SHAPE_ASPECT('','',#21,.T.);#36=SHAPE_ASPECT('','',#21,.T.);\
             #55=(SHAPE_ASPECT_RELATIONSHIP('','',#35,#36)SHAPE_DEFINING_RELATIONSHIP());\
             #56=FEATURE_COMPONENT_RELATIONSHIP('','',#35,#31);\
             #37=DIMENSIONAL_SIZE(#31,'diameter');{}{}",
            gisu(40, 31, 1),
            gisu(41, 36, 2),
            gisu(42, 33, 3)
        );
        let (graph, index) = read(&data);
        let datum_feature = index.resolve(graph, 30);
        assert_eq!(
            reasons(&datum_feature),
            [(
                30,
                UnresolvedReason::NotFollowed {
                    relationship: 50,
                    related: 31
                }
            )]
        );
        assert_eq!(datum_feature.items, []);
        assert_eq!(
            reasons(&index.resolve(graph, 21)),
            [(21, UnresolvedReason::WholeShape)]
        );
        assert_eq!(
            reasons(&index.resolve(graph, 32)),
            [(32, UnresolvedReason::NoUsage)]
        );
        assert_eq!(
            reasons(&index.resolve(graph, 33)),
            [(33, UnresolvedReason::ComponentPath)]
        );
        assert_eq!(
            reasons(&index.resolve(graph, 34)),
            [(34, UnresolvedReason::UnsupportedUsage { usage: 44 })]
        );
        let presentation: Vec<ItemUsageKind> = index
            .usages()
            .iter()
            .filter(|u| u.definition == 32)
            .map(|u| u.kind)
            .collect();
        assert_eq!(presentation, [ItemUsageKind::Presentation; 2]);
        let composite = index.resolve(graph, 35);
        assert_eq!(
            reasons(&composite),
            [
                (
                    35,
                    UnresolvedReason::UnsupportedRelationship {
                        relationship: 55,
                        entity: "SHAPE_ASPECT_RELATIONSHIP+SHAPE_DEFINING_RELATIONSHIP".to_owned()
                    }
                ),
                (
                    35,
                    UnresolvedReason::UnsupportedRelationship {
                        relationship: 56,
                        entity: "FEATURE_COMPONENT_RELATIONSHIP".to_owned()
                    }
                ),
            ]
        );
        assert_eq!(
            reasons(&index.resolve(graph, 37)),
            [(37, UnresolvedReason::UnsupportedDefinition)]
        );
        assert_eq!(
            reasons(&index.resolve(graph, 99)),
            [(99, UnresolvedReason::Missing)]
        );
        assert_eq!(index.naming(3), [], "a component path names nothing");
    }

    #[test]
    fn cycles_depth_and_missing_items() {
        // A component that is not defined, though a usage names it, lists
        // nothing (found by fuzzing: resolve listed its items, naming not).
        let (graph, index) = read(&format!(
            "{BASE}#30=COMPOSITE_SHAPE_ASPECT('','',#21,.T.);\
             #50=SHAPE_ASPECT_RELATIONSHIP('','',#30,#32);{}",
            gisu(40, 32, 1)
        ));
        let undefined = index.resolve(graph, 30);
        assert_eq!(undefined.items, []);
        assert_eq!(reasons(&undefined), [(32, UnresolvedReason::Missing)]);
        assert_eq!(index.naming(1), []);

        let (graph, index) = read(&format!(
            "{BASE}#30=COMPOSITE_SHAPE_ASPECT('','',#21,.T.);#31=COMPOSITE_SHAPE_ASPECT('','',#21,.T.);\
             #50=SHAPE_ASPECT_RELATIONSHIP('','',#30,#31);#51=SHAPE_ASPECT_RELATIONSHIP('','',#31,#30);{}{}",
            gisu(40, 31, 1),
            gisu(41, 30, 99)
        ));
        let cycle = index.resolve(graph, 30);
        assert_eq!(ids(&cycle), [1]);
        assert_eq!(
            reasons(&cycle),
            [
                (
                    30,
                    UnresolvedReason::MissingItem {
                        usage: 41,
                        item: 99
                    }
                ),
                (30, UnresolvedReason::Cycle),
            ]
        );

        let mut chain = BASE.to_owned();
        for k in 0..70u64 {
            write!(
                chain,
                "#{}=COMPOSITE_SHAPE_ASPECT('','',#21,.T.);#{}=SHAPE_ASPECT_RELATIONSHIP('','',#{},#{});",
                1000 + k,
                2000 + k,
                1000 + k,
                1001 + k
            )
            .unwrap();
        }
        chain += &gisu(40, 1070, 1);
        let (graph, index) = read(&chain);
        let deep = index.resolve(graph, 1000);
        assert_eq!(deep.items, []);
        assert_eq!(reasons(&deep), [(1064, UnresolvedReason::TooDeep)]);
    }

    /// What the review of this resolver found: a between aspect's bounds, a
    /// common datum's components, a complex target feature relationship, a
    /// relationship whose related side cannot be read, and a cycle's route.
    #[test]
    fn review_cases() {
        let data = format!(
            "{BASE}#30=BETWEEN_SHAPE_ASPECT('','',#21,.T.);#31=SHAPE_ASPECT('','',#21,.T.);\
             #32=SHAPE_ASPECT('','',#21,.T.);#50=SHAPE_ASPECT_RELATIONSHIP('start feature','',#30,#31);\
             #51=SHAPE_ASPECT_RELATIONSHIP('end feature','',#30,#32);{}{}{}\
             #33=DATUM('',$,#21,.F.,'A');#34=DATUM('',$,#21,.F.,'B');#35=COMMON_DATUM('',$,#21,.F.,'A-B');\
             #36=DATUM_FEATURE('','',#21,.T.);#37=DATUM_FEATURE('','',#21,.T.);\
             #52=SHAPE_ASPECT_RELATIONSHIP('','',#36,#33);#53=SHAPE_ASPECT_RELATIONSHIP('','',#37,#34);\
             #54=SHAPE_ASPECT_RELATIONSHIP('','',#35,#33);#55=SHAPE_ASPECT_RELATIONSHIP('','',#35,#34);{}{}\
             #38=DATUM('',$,#21,.F.,'C');#39=PLACED_DATUM_TARGET_FEATURE('','point',#21,.T.,'1');\
             #40=SHAPE_ASPECT('','',#21,.T.);#56=SHAPE_ASPECT_RELATIONSHIP('','',#39,#38);\
             #57=(FEATURE_FOR_DATUM_TARGET_RELATIONSHIP()SHAPE_ASPECT_RELATIONSHIP('','',#40,#39));{}{}\
             #41=COMPOSITE_SHAPE_ASPECT('','',#21,.T.);#58=SHAPE_ASPECT_RELATIONSHIP('','',#41,$);{}",
            gisu(60, 30, 3),
            gisu(61, 31, 1),
            gisu(62, 32, 2),
            gisu(63, 36, 1),
            gisu(64, 37, 2),
            gisu(65, 39, 6),
            gisu(66, 40, 4),
            gisu(67, 41, 3)
        );
        let (graph, index) = read(&data);
        // The region between the start and end features, not the features.
        let between = index.resolve(graph, 30);
        assert_eq!(ids(&between), [3]);
        assert!(between.is_complete());
        // A common datum's components are datums, each with its feature;
        // neither datum reports the link from the common datum.
        let common = index.resolve(graph, 35);
        assert!(common.is_complete(), "{:?}", common.unresolved);
        assert_eq!(ids(&common), [1, 2]);
        assert_eq!(
            common.items[0].route_kinds,
            [StepKinds::NONE
                .with(StepKind::Component)
                .with(StepKind::DatumFeature)]
        );
        assert!(index.resolve(graph, 33).is_complete());
        // A complex target feature relationship is reported, not dropped.
        let target = index.resolve(graph, 38);
        assert_eq!(ids(&target), [6]);
        assert_eq!(
            reasons(&target),
            [(
                39,
                UnresolvedReason::UnsupportedRelationship {
                    relationship: 57,
                    entity: "FEATURE_FOR_DATUM_TARGET_RELATIONSHIP+SHAPE_ASPECT_RELATIONSHIP"
                        .to_owned()
                }
            )]
        );
        // So is a relationship whose related side is not a reference.
        assert_eq!(
            reasons(&index.resolve(graph, 41)),
            [(
                41,
                UnresolvedReason::UnsupportedRelationship {
                    relationship: 58,
                    entity: "SHAPE_ASPECT_RELATIONSHIP".to_owned()
                }
            )]
        );
        // naming lists the same, with the same kinds of route.
        for aspect in 30..=41 {
            for item in index.resolve(graph, aspect).items {
                let naming = index.naming(item.item);
                let found = naming.iter().find(|n| n.definition == aspect);
                assert_eq!(
                    found.map(|n| &n.route_kinds),
                    Some(&item.route_kinds),
                    "#{aspect}"
                );
            }
        }

        // A cycle's route ends at the aspect it returns to.
        let (graph, index) = read(&format!(
            "{BASE}#30=COMPOSITE_SHAPE_ASPECT('','',#21,.T.);#31=COMPOSITE_SHAPE_ASPECT('','',#21,.T.);\
             #50=SHAPE_ASPECT_RELATIONSHIP('','',#30,#31);#51=SHAPE_ASPECT_RELATIONSHIP('','',#31,#30);{}",
            gisu(40, 31, 1)
        ));
        let cycle = index.resolve(graph, 30);
        let back = cycle
            .unresolved
            .iter()
            .find(|u| u.reason == UnresolvedReason::Cycle)
            .unwrap();
        assert_eq!(back.aspect, 30);
        assert_eq!(back.via.last().map(|step| step.aspect), Some(30));
    }

    /// Layers of two composites, each relating to both below: 2^22 routes,
    /// read in milliseconds, each item once.
    #[test]
    fn diamonds_take_linear_time() {
        let mut data = BASE.to_owned();
        let layers = 22u64;
        for layer in 0..layers {
            for side in 0..2 {
                let aspect = 1000 + layer * 2 + side;
                write!(data, "#{aspect}=COMPOSITE_SHAPE_ASPECT('','',#21,.T.);").unwrap();
                for below in 0..2 {
                    // The last layer relates to the one aspect at the bottom.
                    let to = if layer + 1 == layers {
                        1000 + layers * 2
                    } else {
                        1002 + layer * 2 + below
                    };
                    write!(
                        data,
                        "#{}=SHAPE_ASPECT_RELATIONSHIP('','',#{aspect},#{to});",
                        5000 + layer * 4 + side * 2 + below
                    )
                    .unwrap();
                }
            }
        }
        write!(data, "#{}=SHAPE_ASPECT('','',#21,.T.);", 1000 + layers * 2).unwrap();
        data += &gisu(40, 1000 + layers * 2, 1);
        let (graph, index) = read(&data);
        let start = std::time::Instant::now();
        let top = index.resolve(graph, 1000);
        assert_eq!(ids(&top), [1]);
        assert!(top.is_complete());
        assert_eq!(index.naming(1).len(), 2 * 22 + 1);
        assert!(start.elapsed().as_millis() < 500, "{:?}", start.elapsed());
    }

    /// `naming` is the inverse of `resolve` over every aspect of the
    /// samples above.
    #[test]
    fn naming_inverts_resolve() {
        let data = format!(
            "{BASE}#30=COMPOSITE_GROUP_SHAPE_ASPECT('','',#21,.T.);#31=COMPOSITE_SHAPE_ASPECT('','',#21,.T.);\
             #32=SHAPE_ASPECT('','',#21,.T.);#33=DATUM('',$,#21,.F.,'A');#34=DATUM_FEATURE('','',#21,.T.);\
             #50=SHAPE_ASPECT_RELATIONSHIP('','',#30,#31);#51=SHAPE_ASPECT_RELATIONSHIP('','',#31,#32);\
             #52=SHAPE_ASPECT_RELATIONSHIP('','',#34,#33);#35=CENTRE_OF_SYMMETRY('','',#21,.T.);\
             #53=SHAPE_ASPECT_DERIVING_RELATIONSHIP('','',#35,#30);{}{}{}",
            gisu(40, 32, 1),
            gisu(41, 34, 2),
            gisu(42, 35, 5)
        );
        let (graph, index) = read(&data);
        let mut forward = std::collections::BTreeSet::new();
        for aspect in 30..=35 {
            for item in index.resolve(graph, aspect).items {
                forward.insert((item.item, aspect, item.route_kinds));
            }
        }
        let mut backward = std::collections::BTreeSet::new();
        for item in [1, 2, 5] {
            for naming in index.naming(item) {
                backward.insert((item, naming.definition, naming.route_kinds));
            }
        }
        assert_eq!(forward, backward);
    }

    /// Every list of types is exactly that type and its subtypes in the
    /// four schemas, and every relationship type keeps its relating and
    /// related aspects where the walk reads them (`tools/fetch-schemas.sh`;
    /// skipped without them, failed with `STEPQ_REQUIRE_FIXTURES=1`).
    #[test]
    fn type_lists_and_positions_match_the_schemas() {
        use crate::express::Schema;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/schemas");
        let lists: &[(&str, &[&str])] = &[
            ("SHAPE_ASPECT_RELATIONSHIP", RELATIONSHIPS),
            ("DIMENSIONAL_LOCATION", LOCATIONS),
            ("DIMENSIONAL_SIZE", SIZES),
            ("ITEM_IDENTIFIED_REPRESENTATION_USAGE", USAGES),
            ("DRAUGHTING_MODEL_ITEM_ASSOCIATION", PRESENTATION),
            ("COMPOSITE_SHAPE_ASPECT", COMPOSITES),
            ("DERIVED_SHAPE_ASPECT", DERIVED),
            ("DATUM_FEATURE", DATUM_FEATURES),
            ("DATUM_TARGET", DATUM_TARGETS),
            ("DATUM", DATUMS),
        ];
        let mut found: Vec<HashSet<String>> = vec![HashSet::new(); lists.len()];
        let mut read = 0;
        for file in [
            "ap242e4_mim_lf.exp",
            "ap214e3.exp",
            "ap203e2_mim_lf.exp",
            "ap203.exp",
        ] {
            let Ok(src) = std::fs::read(dir.join(file)) else {
                continue;
            };
            read += 1;
            let schema = Schema::parse(&src).unwrap();
            for ((root, _), found) in lists.iter().zip(&mut found) {
                if schema.entity(root).is_none() {
                    continue;
                }
                let mut seen = HashSet::from([(*root).to_owned()]);
                let mut stack = vec![(*root).to_owned()];
                while let Some(name) = stack.pop() {
                    for entity in schema.entities() {
                        if entity
                            .supertypes
                            .iter()
                            .any(|s| s.eq_ignore_ascii_case(&name))
                            && seen.insert(entity.name.clone())
                        {
                            stack.push(entity.name.clone());
                        }
                    }
                }
                found.extend(seen);
            }
            for name in RELATIONSHIPS {
                let Some(slots) = schema.slots(name) else {
                    continue;
                };
                let offset = RELATIONSHIP_OFFSETS
                    .iter()
                    .find(|(entity, _)| entity == name)
                    .map_or(0, |(_, offset)| *offset);
                let at = |i: usize| slots.get(offset + i).map(|slot| slot.name.as_str());
                assert_eq!(
                    (at(2), at(3)),
                    (Some("RELATING_SHAPE_ASPECT"), Some("RELATED_SHAPE_ASPECT")),
                    "{file}: {name}"
                );
            }
        }
        let required =
            std::env::var_os("STEPQ_REQUIRE_FIXTURES").is_some_and(|v| v != "0" && !v.is_empty());
        if read < 4 {
            assert!(
                !required,
                "{read} of 4 schemas (run tools/fetch-schemas.sh)"
            );
            return;
        }
        for ((root, list), found) in lists.iter().zip(&found) {
            let listed: HashSet<String> = list.iter().map(|&name| name.to_owned()).collect();
            assert_eq!(&listed, found, "{root}");
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn resolutions_have_unique_keys() {
        use crate::testing::unique;
        let (graph, index) = read(&format!(
            "{BASE}#30=COMPOSITE_SHAPE_ASPECT('','',#21,.T.);#31=SHAPE_ASPECT('','',#21,.T.);\
             #50=SHAPE_ASPECT_RELATIONSHIP('','',#30,#31);#51=SHAPE_ASPECT_RELATIONSHIP('','',#31,#30);{}",
            gisu(40, 31, 1)
        ));
        let json = unique(&index.resolve(graph, 30));
        assert!(json.contains("\"route_kinds\":[[\"component\"]]"), "{json}");
        assert!(json.contains("\"kind\":\"not_followed\""), "{json}");
        unique(&index.usages()[0]);
        unique(&index.naming(1));
        for reason in [
            UnresolvedReason::UnsupportedRelationship {
                relationship: 1,
                entity: "X".to_owned(),
            },
            UnresolvedReason::MissingItem { usage: 1, item: 2 },
        ] {
            assert!(unique(&reason).starts_with("{\"kind\":"));
        }
    }
}
