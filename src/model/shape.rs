//! The shape of a product definition itself: its solids, its other items,
//! and every link not followed. See [`ProductStructure::shape`].

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::placement::{is_finite, mapped_item_transform, not_finite};
use super::{Definition, Graph, Placement, PlacementError, ProductStructure, Usage};
use crate::geometry::Rigid;
use crate::p21::{Exchange, Instance, Param, Record};
use crate::policy::{MAX_SHAPE_DEPTH, MAX_SHAPE_VISITS};
use crate::record;

/// What a definition's own shape representations hold, from
/// [`ProductStructure::shape`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct DefinitionShape {
    /// Every solid: an item that is, or has a partial that is, a
    /// `manifold_solid_brep`, `brep_with_voids` or `faceted_brep`, in walk
    /// order.
    pub solids: Vec<ShapeItem>,
    /// Every other item, with its type: surface models, wireframes,
    /// geometric sets, tessellated items. Placements
    /// (`axis2_placement_3d`) are not listed.
    pub other: Vec<ShapeItem>,
    /// Every link that may hold more of the shape but was not followed,
    /// each with the reason.
    pub not_followed: Vec<NotFollowed>,
}

impl DefinitionShape {
    /// True if no link was left unfollowed: [`solids`](Self::solids) and
    /// [`other`](Self::other) are the whole shape the file states for the
    /// definition. An empty, complete shape states no geometry.
    pub fn is_complete(&self) -> bool {
        self.not_followed.is_empty()
    }
}

/// One item of a definition's shape, and the links that reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ShapeItem {
    /// The item's `#id`, such as a `manifold_solid_brep`'s, for
    /// [`topology::Solid::read`](crate::topology::Solid::read).
    pub item: u64,
    /// Its upper-cased entity types, joined with `+`.
    pub entity: String,
    /// The representation that lists it.
    pub representation: u64,
    /// That representation's `context_of_items`, whose units its values are
    /// stated in.
    pub context: u64,
    /// The definition's shape representation it was reached from: the
    /// space an assembly placement of the definition maps. An item reached
    /// from two of them is listed once for each.
    pub shape_representation: u64,
    /// The links from [`shape_representation`](Self::shape_representation)
    /// to [`representation`](Self::representation), outermost first; empty
    /// if the shape representation lists the item itself.
    pub path: Vec<ShapeLink>,
}

impl ShapeItem {
    /// True if a mapped item on [`path`](Self::path) places the item, so
    /// that its coordinates are not those of its shape representation.
    pub fn is_transformed(&self) -> bool {
        self.path
            .iter()
            .any(|link| matches!(link, ShapeLink::MappedItem { .. }))
    }

    /// The map from [`representation`](Self::representation)'s coordinates
    /// (mm) to [`shape_representation`](Self::shape_representation)'s (mm):
    /// `M(target) · M(origin)⁻¹` for each mapped item on the path, outermost
    /// first, each origin read in its map's representation's units and each
    /// target in those of the representation listing the mapped item, as
    /// [`Placements`](super::Placements) reads a usage placed by a mapped
    /// item. The identity if the path holds no mapped item. `graph` must be
    /// the graph the shape was read from.
    ///
    /// # Errors
    ///
    /// A [`PlacementError`] naming the mapped item where it names a usage:
    /// a frame that cannot be read, one whose default x axis ISO 10303 and
    /// Open CASCADE build differently
    /// ([`Divergent`](PlacementError::Divergent)), or a composition that is
    /// not finite.
    pub fn transform(&self, graph: &Graph<'_>) -> Result<Rigid, PlacementError> {
        let mut transform = Rigid::IDENTITY;
        let mut holder = self.shape_representation;
        for link in &self.path {
            match *link {
                ShapeLink::Relationship { representation, .. } => holder = representation,
                ShapeLink::MappedItem {
                    mapped_item,
                    origin,
                    target,
                    representation,
                    ..
                } => {
                    let placed = mapped_item_transform(
                        graph,
                        mapped_item,
                        (origin, representation),
                        (target, holder),
                    )?;
                    transform = transform * placed;
                    if !is_finite(&transform) {
                        return Err(not_finite(graph.exchange(), mapped_item, target));
                    }
                    holder = representation;
                }
            }
        }
        Ok(transform)
    }
}

/// A link followed from one representation to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[non_exhaustive]
pub enum ShapeLink {
    /// A `shape_representation_relationship` with no transformation, that no
    /// assembly usage names, between two representations of one context:
    /// one shape in one space.
    Relationship {
        /// The relationship's `#id`.
        relationship: u64,
        /// The representation it leads to.
        representation: u64,
    },
    /// A `mapped_item`: the items of a representation placed in the one
    /// listing the mapped item.
    MappedItem {
        /// The mapped item's `#id`.
        mapped_item: u64,
        /// Its `mapping_source`, a `representation_map`.
        representation_map: u64,
        /// The map's `mapping_origin`.
        origin: u64,
        /// The mapped item's `mapping_target`.
        target: u64,
        /// The map's `mapped_representation`, which the link leads to.
        representation: u64,
    },
}

/// A link that may lead to more of a definition's shape, and why it was not
/// followed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct NotFollowed {
    /// The relationship, mapped item, reference or representation not
    /// followed.
    pub instance: u64,
    /// Its upper-cased entity types, joined with `+`; empty if it is not
    /// defined.
    pub entity: String,
    /// Why it was not followed.
    pub reason: NotFollowedReason,
}

/// Why a link was not followed; see [`NotFollowed`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[non_exhaustive]
pub enum NotFollowedReason {
    /// The shape is stated in another file: an `applied_document_reference`
    /// to the definition, a document an AP203
    /// `product_definition_with_associated_documents` lists, an 'external
    /// definition' property of one of its
    /// representations or one naming a `document_file` (as `stepq split
    /// --master` writes them), a `shape_representation_reference`, or an
    /// instance defined in an edition 3 `REFERENCE` section.
    External {
        /// The `document_file` named, if one is.
        document: Option<u64>,
    },
    /// A representation relationship with a transformation that no assembly
    /// usage names: no usage says which side is placed.
    TransformedRelationship,
    /// A representation relationship that is not a plain
    /// `shape_representation_relationship`, such as a plain
    /// `representation_relationship`, a subtype, a complex one with more
    /// partials, or one whose `rep_1` and `rep_2` cannot be read.
    NotShapeRelationship,
    /// An item that is a `mapped_item` and more, or a subtype of it (an
    /// annotation, a camera image): not read as a placement of shape.
    NotPlainMappedItem,
    /// A `shape_representation_relationship` between representations of two
    /// context instances, which are two coordinate spaces unless a
    /// transformation relates them.
    DifferentContext {
        /// The two contexts: the one walked from, then the other.
        contexts: [u64; 2],
    },
    /// The link, or the shape representation itself, leads to a
    /// representation other definitions reach too, other than as a
    /// component placement: whose shape it is is not certain.
    OtherDefinition {
        /// Other definitions found to reach it, at most eight, as indices
        /// into [`ProductStructure::definitions`].
        definitions: Vec<usize>,
    },
    /// A link skipped by rule whose far side is not what the rule requires:
    /// a constructive geometry relationship to a representation that is
    /// not constructive geometry, or a relationship an assembly usage names
    /// that leads elsewhere than the usage's other definition.
    UnexpectedTarget {
        /// What the far side must be.
        expected: &'static str,
    },
    /// A mapped item whose representation is already on the path to it
    /// (the schema's `acyclic_mapped_representation`).
    Cycle,
    /// The walk went more than 16 links deep, or visited more than 65,536
    /// representations, items and references to them: crafted input, not
    /// read further.
    TooLarge,
    /// A reference that is not defined, or not the entity the schema
    /// requires there.
    Malformed,
}

/// At most this many other definitions are listed in an
/// [`OtherDefinition`](NotFollowedReason::OtherDefinition), so that many
/// definitions sharing a representation cannot make the lists quadratic.
const LISTED_OWNERS: usize = 8;

/// Which definitions' walks reach a representation; see [`ShapeIndex`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum Owners {
    One(usize),
    /// Two or more; the first [`LISTED_OWNERS`] found.
    Many(Vec<usize>),
}

/// What [`ProductStructure::new`] records for [`ProductStructure::shape`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ShapeIndex {
    /// For every representation some definition's walk reaches, which
    /// definitions reach it.
    owners: HashMap<u64, Owners>,
    /// Each usage's index, by its `#id`.
    usage_by_instance: HashMap<u64, usize>,
    /// The mapped items `ProductStructure` matched to usages, and the
    /// assembly whose component each places.
    component_mapped_items: HashMap<u64, usize>,
}

/// Builds the index: walks every definition's links, as
/// [`ProductStructure::shape`] follows them, recording which definitions
/// reach each representation. A representation becomes owned by one
/// definition, then perhaps by several, and is expanded at each of those two
/// steps only; each relationship, mapped item and context is read once. So
/// the index takes time linear in the file.
pub(super) fn index(graph: &Graph<'_>, definitions: &[Definition], usages: &[Usage]) -> ShapeIndex {
    let mut index = ShapeIndex {
        owners: HashMap::new(),
        usage_by_instance: usages
            .iter()
            .enumerate()
            .map(|(i, usage)| (usage.instance, i))
            .collect(),
        component_mapped_items: usages
            .iter()
            .filter_map(|usage| match usage.placement {
                Some(Placement::MappedItem { mapped_item, .. }) => {
                    Some((mapped_item, usage.parent))
                }
                _ => None,
            })
            .collect(),
    };
    let mut links = Links::new(graph);
    for (definition, defined) in definitions.iter().enumerate() {
        let mut stack: Vec<u64> = defined.shape_representations.clone();
        while let Some(representation) = stack.pop() {
            let expand = match index.owners.entry(representation) {
                Entry::Vacant(entry) => {
                    entry.insert(Owners::One(definition));
                    true
                }
                Entry::Occupied(mut entry) => match entry.get_mut() {
                    Owners::One(owner) if *owner == definition => false,
                    Owners::One(owner) => {
                        let first = *owner;
                        entry.insert(Owners::Many(vec![first, definition]));
                        true
                    }
                    Owners::Many(owners) => {
                        if owners.len() < LISTED_OWNERS && !owners.contains(&definition) {
                            owners.push(definition);
                        }
                        false
                    }
                },
            };
            if expand {
                let components = &index.component_mapped_items;
                stack.extend(links.followed(representation, definition, components));
            }
        }
    }
    index
}

/// The solids, partials included.
const SOLIDS: &[&str] = &[
    "MANIFOLD_SOLID_BREP",
    "BREP_WITH_VOIDS",
    "FACETED_BREP",
    "FACETED_BREP_AND_BREP_WITH_VOIDS",
];

/// `representation_relationship` and its subtypes, in the four schemas.
const RELATIONSHIPS: &[&str] = &[
    "REPRESENTATION_RELATIONSHIP",
    "ANGLE_ASSEMBLY_CONSTRAINT_WITH_DIMENSION",
    "BINARY_ASSEMBLY_CONSTRAINT",
    "COAXIAL_ASSEMBLY_CONSTRAINT",
    "CONSTRAINED_KINEMATIC_MOTION_REPRESENTATION",
    "CONSTRUCTIVE_GEOMETRY_REPRESENTATION_RELATIONSHIP",
    "DEFINITIONAL_REPRESENTATION_RELATIONSHIP",
    "DEFINITIONAL_REPRESENTATION_RELATIONSHIP_WITH_SAME_CONTEXT",
    "DRAPED_ORIENTATION_ANGLE",
    "DRAWING_SHEET_REVISION_SEQUENCE",
    "EVALUATED_CHARACTERISTIC",
    "EVALUATED_CHARACTERISTIC_OF_PRODUCT_AS_INDIVIDUAL_TEST_RESULT",
    "EXPLICIT_PROCEDURAL_REPRESENTATION_RELATIONSHIP",
    "EXPLICIT_PROCEDURAL_SHAPE_REPRESENTATION_RELATIONSHIP",
    "FACE_SHAPE_REPRESENTATION_RELATIONSHIP",
    "FLAT_PATTERN_PLY_REPRESENTATION_RELATIONSHIP",
    "FREE_KINEMATIC_MOTION_REPRESENTATION",
    "GEOMETRY_TO_TOPOLOGY_MODEL_ASSOCIATION",
    "INCIDENCE_ASSEMBLY_CONSTRAINT",
    "ITEM_LINK_MOTION_RELATIONSHIP",
    "KINEMATIC_FRAME_BACKGROUND_REPRESENTATION_ASSOCIATION",
    "KINEMATIC_LINK_REPRESENTATION_ASSOCIATION",
    "LAID_ORIENTATION_ANGLE",
    "LINK_MOTION_RELATIONSHIP",
    "MECHANICAL_DESIGN_AND_DRAUGHTING_RELATIONSHIP",
    "MECHANISM_BASE_PLACEMENT",
    "MOTION_LINK_RELATIONSHIP",
    "PAIR_REPRESENTATION_RELATIONSHIP",
    "PARALLEL_ASSEMBLY_CONSTRAINT",
    "PARALLEL_ASSEMBLY_CONSTRAINT_WITH_DIMENSION",
    "PERPENDICULAR_ASSEMBLY_CONSTRAINT",
    "PLY_ORIENTATION_ANGLE",
    "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION",
    "RESULTING_PATH",
    "SHAPE_DATA_QUALITY_INSPECTED_SHAPE_AND_RESULT_RELATIONSHIP",
    "SHAPE_REPRESENTATION_RELATIONSHIP",
    "SURFACE_DISTANCE_ASSEMBLY_CONSTRAINT_WITH_DIMENSION",
    "TANGENT_ASSEMBLY_CONSTRAINT",
    "TESSELLATED_CONSTRUCTIVE_GEOMETRY_REPRESENTATION_RELATIONSHIP",
    "TOPOLOGY_TO_GEOMETRY_MODEL_ASSOCIATION",
    "VARIATIONAL_CURRENT_REPRESENTATION_RELATIONSHIP",
];

/// `mapped_item` and its subtypes, in the four schemas. Only a plain mapped
/// item is followed; the subtypes (annotations, camera images, fonts) are
/// reported.
const MAPPED_ITEMS: &[&str] = &[
    "MAPPED_ITEM",
    "ANNOTATION_SYMBOL",
    "ANNOTATION_TEXT",
    "ANNOTATION_TEXT_CHARACTER",
    "CAMERA_IMAGE",
    "CAMERA_IMAGE_2D_WITH_SCALE",
    "CAMERA_IMAGE_3D_WITH_SCALE",
    "DIMENSION_TEXT_ASSOCIATIVITY",
    "INCLUDED_TEXT_BLOCK",
    "PATH_AREA_WITH_PARAMETERS",
    "REPOSITIONED_NEUTRAL_SKETCH",
    "USER_DEFINED_CURVE_FONT",
    "USER_DEFINED_MARKER",
    "USER_DEFINED_TERMINATOR_SYMBOL",
];

const CDSR: &str = "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION";
const SRR: &str = "SHAPE_REPRESENTATION_RELATIONSHIP";
const RR: &str = "REPRESENTATION_RELATIONSHIP";
const RRWT: &str = "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION";

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

/// What a link from a representation is.
#[derive(Debug, Clone)]
enum Link {
    /// Followed, to the representation on its far side.
    Follow { far: u64 },
    /// A relationship an assembly usage names, to `far`: the usage's
    /// `#id`, if it can be read.
    Usage { far: u64, usage: Option<u64> },
    /// A constructive geometry relationship, skipped if `far` is
    /// constructive geometry.
    Constructive { far: u64 },
    /// Not followed.
    Refused(NotFollowedReason),
    /// Not a link to more shape.
    None,
}

/// A mapped item, read.
#[derive(Debug, Clone, Copy)]
struct Mapped {
    representation_map: u64,
    origin: u64,
    target: u64,
    representation: u64,
}

/// What an item is to the walk.
#[derive(Debug, Clone, Copy)]
enum MappedItem {
    /// Not a mapped item.
    No,
    /// A plain mapped item.
    Plain(Mapped),
    /// One whose attributes cannot be read.
    Malformed,
    /// A subtype, or one with other partials (a camera image, an
    /// annotation): not read as a placement of shape.
    Other,
}

/// A representation, read: its items and its context.
type Read = Option<(Rc<[u64]>, u64)>;

/// Reads the links between representations, the same way for the index and
/// for each walk, each relationship, mapped item and representation once.
struct Links<'g, 'a> {
    graph: &'g Graph<'a>,
    representations: HashMap<u64, Read>,
    mapped_items: HashMap<u64, MappedItem>,
    relationships: HashMap<(usize, u64), Link>,
}

impl<'g, 'a> Links<'g, 'a> {
    fn new(graph: &'g Graph<'a>) -> Self {
        Self {
            graph,
            representations: HashMap::new(),
            mapped_items: HashMap::new(),
            relationships: HashMap::new(),
        }
    }

    fn exchange(&self) -> &'g Exchange<'a> {
        self.graph.exchange()
    }

    /// The items and context of `#representation`, or `None` if it cannot
    /// be read as one.
    fn representation(&mut self, representation: u64) -> Read {
        let exchange = self.graph.exchange();
        self.representations
            .entry(representation)
            .or_insert_with(|| {
                record::representation(exchange, representation)
                    .ok()
                    .map(|read| (read.items.into(), read.context))
            })
            .clone()
    }

    /// The representations `#representation` leads to, for definition
    /// `definition`, by links the walk follows: its plain shape
    /// relationships, and the mapped items it lists that are not the
    /// definition's component placements.
    fn followed(
        &mut self,
        representation: u64,
        definition: usize,
        components: &HashMap<u64, usize>,
    ) -> Vec<u64> {
        let mut found = Vec::new();
        if let Some((items, _)) = self.representation(representation) {
            for &item in items.iter() {
                if components.get(&item) == Some(&definition) {
                    continue;
                }
                if let MappedItem::Plain(mapped) = self.mapped_item(item) {
                    found.push(mapped.representation);
                }
            }
        }
        let Some(node) = self.graph.node(representation) else {
            return found;
        };
        for &user in self.graph.referenced_by(node) {
            if let Link::Follow { far } = self.relationship(user, representation) {
                found.push(far);
            }
        }
        found
    }

    /// What `#item` is as a mapped item.
    fn mapped_item(&mut self, item: u64) -> MappedItem {
        if let Some(&read) = self.mapped_items.get(&item) {
            return read;
        }
        let read = self.read_mapped_item(item);
        self.mapped_items.insert(item, read);
        read
    }

    fn read_mapped_item(&self, item: u64) -> MappedItem {
        let exchange = self.exchange();
        let Some(instance) = exchange.get(item) else {
            return MappedItem::No;
        };
        let names = names(exchange, instance);
        if !names
            .iter()
            .any(|name| MAPPED_ITEMS.contains(&name.as_str()))
        {
            return MappedItem::No;
        }
        let params = if instance.is_complex() {
            // A placed representation item, and nothing more.
            let plain = names.iter().all(|name| {
                matches!(
                    name.as_str(),
                    "MAPPED_ITEM" | "REPRESENTATION_ITEM" | "GEOMETRIC_REPRESENTATION_ITEM"
                )
            });
            if !plain {
                return MappedItem::Other;
            }
            if record::repeats_partial(exchange, instance) {
                return MappedItem::Malformed;
            }
            record::partial(exchange, instance, "MAPPED_ITEM")
                .and_then(|r| record::exact_params(r, 2))
        } else {
            if names != ["MAPPED_ITEM"] {
                return MappedItem::Other;
            }
            exchange
                .records(instance)
                .next()
                .and_then(|record| record::exact_params(record, 3))
                .map(|params| params[1..].to_vec())
        };
        match self.mapped(params) {
            Some(mapped) => MappedItem::Plain(mapped),
            None => MappedItem::Malformed,
        }
    }

    fn mapped(&self, params: Option<Vec<Param<'_>>>) -> Option<Mapped> {
        let params = params?;
        let (map, target) = (params[0].reference()?, params[1].reference()?);
        let exchange = self.exchange();
        let map_instance = exchange.get(map)?;
        if map_instance.is_complex() {
            return None;
        }
        let map_record = exchange
            .records(map_instance)
            .next()
            .filter(|record| record.is("REPRESENTATION_MAP"))?;
        let map_params = record::exact_params(map_record, 2)?;
        Some(Mapped {
            representation_map: map,
            origin: map_params[0].reference()?,
            target,
            representation: map_params[1].reference()?,
        })
    }

    /// What the instance at graph node `user`, which refers to
    /// `#representation`, is as a link from it.
    fn relationship(&mut self, user: usize, representation: u64) -> Link {
        if let Some(link) = self.relationships.get(&(user, representation)) {
            return link.clone();
        }
        let link = self.read_relationship(user, representation);
        self.relationships
            .insert((user, representation), link.clone());
        link
    }

    fn read_relationship(&mut self, user: usize, representation: u64) -> Link {
        let exchange = self.exchange();
        let instance = self.graph.instance(user);
        let names = names(exchange, instance);
        if !names
            .iter()
            .any(|name| RELATIONSHIPS.contains(&name.as_str()))
        {
            return Link::None;
        }
        // rep_1 and rep_2 are attributes 2 and 3 of a relationship's
        // representation_relationship partial, or of a simple record whose
        // supertypes are relationships only. Where they are not, the
        // relationship cannot be read, and is reported.
        let base = if instance.is_complex() {
            record::partial(exchange, instance, RR)
        } else {
            exchange.records(instance).next()
        };
        let sides: Option<(u64, u64)> = base.and_then(|record| {
            let params: Vec<Param<'_>> = record.params().take(4).collect();
            Some((params.get(2)?.reference()?, params.get(3)?.reference()?))
        });
        let far = match sides {
            Some((rep_1, rep_2)) if rep_1 == representation && rep_2 != representation => rep_2,
            Some((rep_1, rep_2)) if rep_2 == representation && rep_1 != representation => rep_1,
            Some((rep_1, rep_2)) if rep_1 == representation && rep_2 == representation => {
                return Link::None;
            }
            _ => return Link::Refused(NotFollowedReason::NotShapeRelationship),
        };
        if let Some(usage) = self.named_by_usage(user) {
            return Link::Usage { far, usage };
        }
        let is = |name: &str| names.iter().any(|n| n == name);
        if is("CONSTRUCTIVE_GEOMETRY_REPRESENTATION_RELATIONSHIP")
            || is("TESSELLATED_CONSTRUCTIVE_GEOMETRY_REPRESENTATION_RELATIONSHIP")
        {
            return Link::Constructive { far };
        }
        if is(RRWT) {
            return Link::Refused(NotFollowedReason::TransformedRelationship);
        }
        let plain = if instance.is_complex() {
            let mut partials: Vec<&str> = names.iter().map(String::as_str).collect();
            partials.sort_unstable();
            partials == [RR, SRR]
                && record::partial(exchange, instance, RR)
                    .and_then(|r| record::exact_params(r, 4))
                    .is_some()
                && record::partial(exchange, instance, SRR)
                    .and_then(|r| record::exact_params(r, 0))
                    .is_some()
        } else {
            names == [SRR] && base.and_then(|r| record::exact_params(r, 4)).is_some()
        };
        if !plain {
            return Link::Refused(NotFollowedReason::NotShapeRelationship);
        }
        let here = self
            .representation(representation)
            .map(|(_, context)| context);
        let there = self.representation(far).map(|(_, context)| context);
        match (here, there) {
            (Some(here), Some(there)) if here == there => Link::Follow { far },
            (Some(here), Some(there)) => Link::Refused(NotFollowedReason::DifferentContext {
                contexts: [here, there],
            }),
            (_, None) => Link::Refused(self.unreadable(far)),
            (None, _) => Link::Refused(NotFollowedReason::Malformed),
        }
    }

    /// If a `context_dependent_shape_representation` names the relationship
    /// at graph node `relationship`: the `#id` of the usage its product
    /// definition shape is of, if that can be read.
    #[expect(
        clippy::option_option,
        reason = "named by none, or by one whose usage cannot be read"
    )]
    fn named_by_usage(&self, relationship: usize) -> Option<Option<u64>> {
        let exchange = self.exchange();
        let link = self
            .graph
            .referenced_by(relationship)
            .iter()
            .find(|&&user| {
                exchange
                    .records(self.graph.instance(user))
                    .any(|record| record.is(CDSR))
            })?;
        let shape = record::reference(
            exchange
                .records(self.graph.instance(*link))
                .next()
                .and_then(|record| record.param(1))
                .as_ref(),
        );
        let usage = shape
            .and_then(|shape| exchange.get(shape))
            .and_then(|instance| exchange.records(instance).next())
            .and_then(|record| record::reference(record.param(2).as_ref()));
        Some(usage)
    }

    /// Why `#id`, where a representation must be, is not followed.
    fn unreadable(&self, id: u64) -> NotFollowedReason {
        let exchange = self.exchange();
        let reference = exchange.get(id).is_some_and(|instance| {
            exchange
                .records(instance)
                .any(|record| record.is("SHAPE_REPRESENTATION_REFERENCE"))
        });
        if reference || exchange.is_external(id) {
            NotFollowedReason::External { document: None }
        } else {
            NotFollowedReason::Malformed
        }
    }
}

impl ProductStructure {
    /// The shape of definition `definition` itself, read from its shape
    /// representations: its solids, its other items, and every link not
    /// followed. Never fails: what cannot be followed is listed with the
    /// reason, so an empty, [complete](DefinitionShape::is_complete) shape
    /// states no geometry and an incomplete one says stepq cannot tell.
    ///
    /// From each of the definition's
    /// [`shape_representations`](super::Definition::shape_representations),
    /// the walk reads the items, and follows:
    ///
    /// * a `shape_representation_relationship` with no transformation, simple
    ///   or with exactly the `representation_relationship` and
    ///   `shape_representation_relationship` partials, that no
    ///   `context_dependent_shape_representation` names and whose two sides
    ///   name one context instance, from whichever side it is on;
    /// * a plain `mapped_item` that is not one this structure matched to a
    ///   usage of this definition, into its map's representation.
    ///
    /// It skips, as not this definition's own shape: `axis2_placement_3d`
    /// items (frames); a relationship an assembly usage names, if it leads
    /// to a shape representation of the usage's other definition; its
    /// components' mapped items; a constructive geometry relationship
    /// (supplemental geometry), if it leads to constructive geometry; and
    /// the `representation_map`s that point back. Every other item is a
    /// solid or listed in [`other`](DefinitionShape::other); every other
    /// link that may hold more shape is listed in
    /// [`not_followed`](DefinitionShape::not_followed), as is a
    /// representation another definition also reaches, a definition whose
    /// shape is in another file, and a walk that grows too large. Only
    /// `#id`s and entity types are read.
    ///
    /// The walk visits at most 65,536 representations, items and references
    /// to representations, and each relationship, mapped item and
    /// representation is read once, so one call takes time linear in the
    /// file. `graph` must be the graph this structure was read from.
    ///
    /// # Panics
    ///
    /// Panics if `definition` is out of range.
    pub fn shape(&self, graph: &Graph<'_>, definition: usize) -> DefinitionShape {
        let defined = &self.definitions()[definition];
        let mut walk = Walk {
            links: Links::new(graph),
            structure: self,
            index: self.shape_index(),
            definition,
            out: DefinitionShape::default(),
            reported: HashSet::new(),
            visits: 0,
            exhausted: false,
            seen: HashSet::new(),
        };
        walk.documents(defined.instance);
        let mut roots = Vec::new();
        for &root in &defined.shape_representations {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        for root in roots {
            if let Some(others) = walk.others(root) {
                walk.refuse(
                    root,
                    NotFollowedReason::OtherDefinition {
                        definitions: others,
                    },
                );
                continue;
            }
            walk.seen.clear();
            let mut at = At {
                root,
                path: Vec::new(),
                on_path: vec![root],
                mapped: Vec::new(),
            };
            walk.visit(&mut at, root);
        }
        walk.out
    }
}

/// Where the walk is: its root, the links from it, the representations on
/// them, and the mapped items among them.
struct At {
    root: u64,
    path: Vec<ShapeLink>,
    on_path: Vec<u64>,
    mapped: Vec<u64>,
}

struct Walk<'s, 'g, 'a> {
    links: Links<'g, 'a>,
    structure: &'s ProductStructure,
    index: &'s ShapeIndex,
    definition: usize,
    out: DefinitionShape,
    /// The instances already listed as not followed, with the kind of
    /// reason.
    reported: HashSet<(u64, std::mem::Discriminant<NotFollowedReason>)>,
    visits: usize,
    exhausted: bool,
    /// The representations and items visited from the current root, each
    /// with the mapped items that place it.
    seen: HashSet<(u64, Vec<u64>)>,
}

impl Walk<'_, '_, '_> {
    fn refuse(&mut self, instance: u64, reason: NotFollowedReason) {
        if self
            .reported
            .insert((instance, std::mem::discriminant(&reason)))
        {
            let entity = entity(self.links.exchange(), instance);
            self.out.not_followed.push(NotFollowed {
                instance,
                entity,
                reason,
            });
        }
    }

    /// Counts a visit; false once the walk is too large, reported at
    /// `#instance`.
    fn count(&mut self, instance: u64) -> bool {
        if self.exhausted {
            return false;
        }
        self.visits += 1;
        if self.visits > MAX_SHAPE_VISITS {
            self.exhausted = true;
            self.refuse(instance, NotFollowedReason::TooLarge);
            return false;
        }
        true
    }

    /// The other definitions that reach `#representation`, if any do.
    fn others(&self, representation: u64) -> Option<Vec<usize>> {
        match self.index.owners.get(&representation) {
            Some(Owners::Many(owners)) => Some(
                owners
                    .iter()
                    .copied()
                    .filter(|&owner| owner != self.definition)
                    .collect(),
            ),
            Some(Owners::One(owner)) if *owner != self.definition => Some(vec![*owner]),
            _ => None,
        }
    }

    /// Reports the documents the definition `#definition` is stated in: each
    /// `applied_document_reference` that names it, and the documents an
    /// AP203 `product_definition_with_associated_documents` lists.
    fn documents(&mut self, definition: u64) {
        let graph = self.links.graph;
        let exchange = graph.exchange();
        let Some(node) = graph.node(definition) else {
            return;
        };
        let instance = graph.instance(node);
        let associated = (!instance.is_complex())
            .then(|| exchange.records(instance).next())
            .flatten()
            .filter(|record| record.is("PRODUCT_DEFINITION_WITH_ASSOCIATED_DOCUMENTS"))
            .and_then(|record| record.param(4))
            .and_then(|documents| documents.list());
        for document in associated
            .into_iter()
            .flatten()
            .filter_map(|p| p.reference())
        {
            self.refuse(
                document,
                NotFollowedReason::External {
                    document: Some(document),
                },
            );
        }
        for &user in graph.referenced_by(node) {
            let instance = graph.instance(user);
            if instance.is_complex() {
                continue;
            }
            let Some(record) = exchange.records(instance).next() else {
                continue;
            };
            if record.is("APPLIED_DOCUMENT_REFERENCE") {
                let document = record::reference(record.param(0).as_ref());
                self.refuse(instance.id, NotFollowedReason::External { document });
            }
        }
    }

    fn visit(&mut self, at: &mut At, representation: u64) {
        if !self.count(representation) || !self.seen.insert((representation, at.mapped.clone())) {
            return;
        }
        let Some((items, context)) = self.links.representation(representation) else {
            let reason = self.links.unreadable(representation);
            self.refuse(representation, reason);
            return;
        };
        for &item in items.iter() {
            if !self.count(item) {
                return;
            }
            self.item(at, representation, context, item);
        }
        let graph = self.links.graph;
        let Some(node) = graph.node(representation) else {
            return;
        };
        for &user in graph.referenced_by(node) {
            if !self.count(representation) {
                return;
            }
            self.link(at, representation, user);
        }
    }

    fn item(&mut self, at: &mut At, representation: u64, context: u64, item: u64) {
        let exchange = self.links.exchange();
        let Some(instance) = exchange.get(item) else {
            let reason = self.links.unreadable(item);
            self.refuse(item, reason);
            return;
        };
        let names = names(exchange, instance);
        let component = self.index.component_mapped_items.get(&item) == Some(&self.definition);
        if names == ["AXIS2_PLACEMENT_3D"] || component {
            return;
        }
        match self.links.mapped_item(item) {
            MappedItem::Plain(mapped) => return self.mapped(at, item, &mapped),
            MappedItem::Malformed => return self.refuse(item, NotFollowedReason::Malformed),
            MappedItem::Other => return self.refuse(item, NotFollowedReason::NotPlainMappedItem),
            MappedItem::No => {}
        }
        if !self.seen.insert((item, at.mapped.clone())) {
            return;
        }
        let shape_item = ShapeItem {
            item,
            entity: names.join("+"),
            representation,
            context,
            shape_representation: at.root,
            path: at.path.clone(),
        };
        if names.iter().any(|name| SOLIDS.contains(&name.as_str())) {
            self.out.solids.push(shape_item);
        } else {
            self.out.other.push(shape_item);
        }
    }

    fn mapped(&mut self, at: &mut At, item: u64, mapped: &Mapped) {
        if let Some(others) = self.others(mapped.representation) {
            return self.refuse(
                item,
                NotFollowedReason::OtherDefinition {
                    definitions: others,
                },
            );
        }
        if at.on_path.contains(&mapped.representation) {
            return self.refuse(item, NotFollowedReason::Cycle);
        }
        if at.path.len() >= MAX_SHAPE_DEPTH {
            return self.refuse(item, NotFollowedReason::TooLarge);
        }
        let next = mapped.representation;
        at.path.push(ShapeLink::MappedItem {
            mapped_item: item,
            representation_map: mapped.representation_map,
            origin: mapped.origin,
            target: mapped.target,
            representation: next,
        });
        at.on_path.push(next);
        at.mapped.push(item);
        self.visit(at, next);
        at.mapped.pop();
        at.on_path.pop();
        at.path.pop();
    }

    /// Follows, skips or reports the instance at graph node `user`, which
    /// refers to `#representation`.
    fn link(&mut self, at: &mut At, representation: u64, user: usize) {
        let graph = self.links.graph;
        let id = graph.instance(user).id;
        if self.external_property(user, id) {
            return;
        }
        match self.links.relationship(user, representation) {
            Link::None => {}
            Link::Refused(reason) => self.refuse(id, reason),
            Link::Usage { far, usage } => {
                let other = usage
                    .and_then(|usage| self.index.usage_by_instance.get(&usage))
                    .map(|&usage| &self.structure.usages()[usage])
                    .and_then(|usage| {
                        if usage.parent == self.definition {
                            Some(usage.child)
                        } else if usage.child == self.definition {
                            Some(usage.parent)
                        } else {
                            None
                        }
                    });
                let expected = other.is_some_and(|other| {
                    self.structure.definitions()[other]
                        .shape_representations
                        .contains(&far)
                });
                if !expected {
                    self.refuse(
                        id,
                        NotFollowedReason::UnexpectedTarget {
                            expected: "a shape representation of the usage's other definition",
                        },
                    );
                }
            }
            Link::Constructive { far } => {
                let exchange = self.links.exchange();
                let constructive = exchange.get(far).is_some_and(|instance| {
                    exchange.records(instance).any(|record| {
                        record::upper(record.name())
                            .ends_with("CONSTRUCTIVE_GEOMETRY_REPRESENTATION")
                    })
                });
                if !constructive {
                    self.refuse(
                        id,
                        NotFollowedReason::UnexpectedTarget {
                            expected: "constructive_geometry_representation",
                        },
                    );
                }
            }
            Link::Follow { far } => {
                if at.on_path.contains(&far) {
                    return;
                }
                if let Some(others) = self.others(far) {
                    return self.refuse(
                        id,
                        NotFollowedReason::OtherDefinition {
                            definitions: others,
                        },
                    );
                }
                if at.path.len() >= MAX_SHAPE_DEPTH {
                    return self.refuse(id, NotFollowedReason::TooLarge);
                }
                at.path.push(ShapeLink::Relationship {
                    relationship: id,
                    representation: far,
                });
                at.on_path.push(far);
                self.visit(at, far);
                at.on_path.pop();
                at.path.pop();
            }
        }
    }

    /// True, after reporting it, if the instance `#id` at graph node `user`
    /// is a `property_definition_representation` stating that the
    /// representation it names is defined in another file: its property is
    /// called 'external definition' or names a `document_file`.
    fn external_property(&mut self, user: usize, id: u64) -> bool {
        let exchange = self.links.exchange();
        let instance = self.links.graph.instance(user);
        let Some(record) = (!instance.is_complex())
            .then(|| exchange.records(instance).next())
            .flatten()
            .filter(|record| record.is("PROPERTY_DEFINITION_REPRESENTATION"))
        else {
            return false;
        };
        let property: Option<Record<'_>> = record::reference(record.param(0).as_ref())
            .and_then(|id| exchange.get(id))
            .filter(|instance| !instance.is_complex())
            .and_then(|instance| exchange.records(instance).next())
            .filter(|record| record.is("PROPERTY_DEFINITION"));
        let Some(property) = property else {
            return false;
        };
        let named = record::text(property.param(0))
            .is_some_and(|name| name.eq_ignore_ascii_case("external definition"));
        let document = record::reference(property.param(2).as_ref()).filter(|&id| {
            exchange
                .get(id)
                .and_then(|instance| exchange.records(instance).next())
                .is_some_and(|record| record.is("DOCUMENT_FILE"))
        });
        if named || document.is_some() {
            self.refuse(id, NotFollowedReason::External { document });
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use crate::p21::parse;

    const HEADER: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('AP242'));ENDSEC;DATA;
#1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
#2=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#1))REPRESENTATION_CONTEXT('',''));
#3=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#1))REPRESENTATION_CONTEXT('',''));
";

    /// A part, #10, whose shape representation is #13.
    const PART: &str = "#10=PRODUCT_DEFINITION('p','',$,$);#11=PRODUCT_DEFINITION_SHAPE('','',#10);
#12=SHAPE_DEFINITION_REPRESENTATION(#11,#13);";

    fn graph(data: &str) -> &'static Graph<'static> {
        let src = format!("{HEADER}{data}\nENDSEC;END-ISO-10303-21;");
        let src: &'static [u8] = Box::leak(src.into_bytes().into_boxed_slice());
        // build, not new: some cases refer to instances that are not defined.
        Box::leak(Box::new(Graph::build(parse(src).unwrap())))
    }

    /// The shape of every definition, by its `#id`.
    fn shapes(data: &str) -> HashMap<u64, DefinitionShape> {
        let graph = graph(data);
        let structure = ProductStructure::new(graph);
        structure
            .definitions()
            .iter()
            .enumerate()
            .map(|(i, d)| (d.instance, structure.shape(graph, i)))
            .collect()
    }

    fn part(items: &str) -> DefinitionShape {
        shapes(&format!("{PART}#13=SHAPE_REPRESENTATION('',({items}),#2);"))
            .remove(&10)
            .unwrap()
    }

    fn ids(items: &[ShapeItem]) -> Vec<u64> {
        items.iter().map(|item| item.item).collect()
    }

    fn reasons(shape: &DefinitionShape) -> Vec<(u64, NotFollowedReason)> {
        shape
            .not_followed
            .iter()
            .map(|n| (n.instance, n.reason.clone()))
            .collect()
    }

    #[test]
    fn solids_listed_by_the_shape_representation() {
        let shape = shapes(&format!(
            "{PART}#13=SHAPE_REPRESENTATION('',(#20,#21,#22,#23,#24,#25),#2);
#20=MANIFOLD_SOLID_BREP('',#90);#21=BREP_WITH_VOIDS('',#90,(#91));
#22=FACETED_BREP('',#90);#23=(MANIFOLD_SOLID_BREP('',#90)REPRESENTATION_ITEM('')SOLID_MODEL());
#24=AXIS2_PLACEMENT_3D('',#92,$,$);#25=SHELL_BASED_SURFACE_MODEL('',(#93));"
        ))
        .remove(&10)
        .unwrap();
        assert!(shape.is_complete());
        assert_eq!(ids(&shape.solids), [20, 21, 22, 23]);
        let solid = &shape.solids[0];
        assert_eq!(solid.entity, "MANIFOLD_SOLID_BREP");
        assert_eq!((solid.representation, solid.shape_representation), (13, 13));
        assert_eq!(solid.context, 2);
        assert!(solid.path.is_empty() && !solid.is_transformed());
        assert_eq!(
            shape.solids[3].entity,
            "MANIFOLD_SOLID_BREP+REPRESENTATION_ITEM+SOLID_MODEL"
        );
        assert_eq!(ids(&shape.other), [25], "the placement is not listed");
        assert_eq!(shape.other[0].entity, "SHELL_BASED_SURFACE_MODEL");
    }

    /// Unigraphics writes the definition's representation as `rep_1`,
    /// Autodesk's exporter as `rep_2`; both are one space.
    #[test]
    fn plain_shape_relationships_either_way() {
        for relationship in [
            "#30=SHAPE_REPRESENTATION_RELATIONSHIP('','',#13,#14);",
            "#30=SHAPE_REPRESENTATION_RELATIONSHIP('','',#14,#13);",
            "#30=(REPRESENTATION_RELATIONSHIP('','',#14,#13)SHAPE_REPRESENTATION_RELATIONSHIP());",
        ] {
            let shape = shapes(&format!(
                "{PART}#13=SHAPE_REPRESENTATION('',(#24),#2);#24=AXIS2_PLACEMENT_3D('',#92,$,$);
#14=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#20,#25),#2);#20=MANIFOLD_SOLID_BREP('',#90);
#25=GEOMETRIC_CURVE_SET('',(#94));{relationship}"
            ))
            .remove(&10)
            .unwrap();
            assert!(shape.is_complete(), "{relationship}: {shape:?}");
            assert_eq!(ids(&shape.solids), [20]);
            let solid = &shape.solids[0];
            assert_eq!((solid.representation, solid.shape_representation), (14, 13));
            assert_eq!(
                solid.path,
                [ShapeLink::Relationship {
                    relationship: 30,
                    representation: 14
                }]
            );
            assert!(!solid.is_transformed());
            assert_eq!(ids(&shape.other), [25]);
        }
    }

    /// Supplemental geometry is skipped, but only when it is constructive
    /// geometry; a tessellated solid is another item.
    #[test]
    fn supplemental_and_tessellated_geometry() {
        let constructive = format!(
            "{PART}#13=SHAPE_REPRESENTATION('',(#20,#26),#2);#20=MANIFOLD_SOLID_BREP('',#90);
#26=TESSELLATED_SOLID('',(#95),$);
#14=CONSTRUCTIVE_GEOMETRY_REPRESENTATION('',(#40,#41),#2);#40=PLANE('',#92);
#41=SHELL_BASED_SURFACE_MODEL('',(#93));
#30=CONSTRUCTIVE_GEOMETRY_REPRESENTATION_RELATIONSHIP('','',#13,#14);"
        );
        let shape = shapes(&constructive).remove(&10).unwrap();
        assert!(shape.is_complete());
        assert_eq!(ids(&shape.solids), [20]);
        assert_eq!(
            ids(&shape.other),
            [26],
            "the constructive plane and shell are not"
        );

        let body = constructive.replace(
            "#14=CONSTRUCTIVE_GEOMETRY_REPRESENTATION(",
            "#14=SHAPE_REPRESENTATION(",
        );
        let shape = shapes(&body).remove(&10).unwrap();
        assert_eq!(
            reasons(&shape),
            [(
                30,
                NotFollowedReason::UnexpectedTarget {
                    expected: "constructive_geometry_representation"
                }
            )]
        );
    }

    /// An assembly and its part, placed by a relationship a usage names:
    /// the assembly's own shape is empty and complete, the part's is its
    /// solid; a usage's relationship that leads elsewhere is reported.
    #[test]
    fn assemblies_hold_no_component_shape() {
        let assembly = format!(
            "{PART}#13=SHAPE_REPRESENTATION('',(#20,#24),#2);#20=MANIFOLD_SOLID_BREP('',#90);
#24=AXIS2_PLACEMENT_3D('',#92,$,$);
#50=PRODUCT_DEFINITION('a','',$,$);#51=PRODUCT_DEFINITION_SHAPE('','',#50);
#52=SHAPE_DEFINITION_REPRESENTATION(#51,#53);#53=SHAPE_REPRESENTATION('',(#54),#2);
#54=AXIS2_PLACEMENT_3D('',#92,$,$);
#60=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','','',#50,#10,$);#61=PRODUCT_DEFINITION_SHAPE('','',#60);
#62=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#63,#61);
#63=(REPRESENTATION_RELATIONSHIP('','',#13,#53)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#64)SHAPE_REPRESENTATION_RELATIONSHIP());
#64=ITEM_DEFINED_TRANSFORMATION('','',#24,#54);"
        );
        let mut shapes_ = shapes(&assembly);
        let assembly_shape = shapes_.remove(&50).unwrap();
        assert_eq!(assembly_shape, DefinitionShape::default());
        let part_shape = shapes_.remove(&10).unwrap();
        assert!(part_shape.is_complete());
        assert_eq!(ids(&part_shape.solids), [20]);

        // The part's own second representation, named by the usage.
        let elsewhere = assembly.replace(
            "REPRESENTATION_RELATIONSHIP('','',#13,#53)",
            "REPRESENTATION_RELATIONSHIP('','',#13,#14)",
        ) + "#14=SHAPE_REPRESENTATION('',(#21),#2);#21=MANIFOLD_SOLID_BREP('',#90);";
        let part_shape = shapes(&elsewhere).remove(&10).unwrap();
        assert_eq!(
            reasons(&part_shape),
            [(
                63,
                NotFollowedReason::UnexpectedTarget {
                    expected: "a shape representation of the usage's other definition"
                }
            )]
        );
    }

    /// An assembly holding its two parts by mapped items that the structure
    /// matched: nothing of the parts is the assembly's.
    #[test]
    fn component_mapped_items_are_not_the_assemblys() {
        let data = format!(
            "{PART}#13=SHAPE_REPRESENTATION('',(#20,#24),#2);#20=MANIFOLD_SOLID_BREP('',#90);
#24=AXIS2_PLACEMENT_3D('',#92,$,$);
#50=PRODUCT_DEFINITION('a','',$,$);#51=PRODUCT_DEFINITION_SHAPE('','',#50);
#52=SHAPE_DEFINITION_REPRESENTATION(#51,#53);#53=SHAPE_REPRESENTATION('',(#54,#70,#71),#2);
#54=AXIS2_PLACEMENT_3D('',#92,$,$);
#60=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','','',#50,#10,$);
#61=NEXT_ASSEMBLY_USAGE_OCCURRENCE('2','','',#50,#10,$);
#69=REPRESENTATION_MAP(#24,#13);#70=MAPPED_ITEM('',#69,#54);#71=MAPPED_ITEM('',#69,#55);
#55=AXIS2_PLACEMENT_3D('',#92,$,$);"
        );
        let mut shapes_ = shapes(&data);
        assert_eq!(shapes_.remove(&50).unwrap(), DefinitionShape::default());
        assert_eq!(ids(&shapes_.remove(&10).unwrap().solids), [20]);
    }

    /// A part that holds one representation of its own twice, by two mapped
    /// items: two solids, each with its path and its transform.
    #[test]
    fn a_parts_own_mapped_items_place_its_solids() {
        let graph = graph(&format!(
            "{PART}#13=SHAPE_REPRESENTATION('',(#70,#71),#2);
#14=SHAPE_REPRESENTATION('',(#20,#24),#2);#20=MANIFOLD_SOLID_BREP('',#90);
#24=AXIS2_PLACEMENT_3D('',#80,$,$);#80=CARTESIAN_POINT('',(0.,0.,0.));
#69=REPRESENTATION_MAP(#24,#14);#70=MAPPED_ITEM('',#69,#24);#71=MAPPED_ITEM('',#69,#25);
#25=AXIS2_PLACEMENT_3D('',#81,$,$);#81=CARTESIAN_POINT('',(100.,0.,0.));"
        ));
        let structure = ProductStructure::new(graph);
        let shape = structure.shape(graph, 0);
        assert!(shape.is_complete());
        assert_eq!(ids(&shape.solids), [20, 20], "once per mapped item");
        let paths: Vec<&ShapeLink> = shape.solids.iter().map(|s| &s.path[0]).collect();
        assert_eq!(
            paths[1],
            &ShapeLink::MappedItem {
                mapped_item: 71,
                representation_map: 69,
                origin: 24,
                target: 25,
                representation: 14
            }
        );
        assert!(shape.solids.iter().all(ShapeItem::is_transformed));
        assert_eq!(shape.solids[0].transform(graph), Ok(Rigid::IDENTITY));
        let moved = shape.solids[1].transform(graph).unwrap();
        assert_eq!(moved.apply_point([1.0, 2.0, 3.0]), [101.0, 2.0, 3.0]);
        assert_eq!(shape.solids[1].representation, 14);
        assert_eq!(shape.solids[1].shape_representation, 13);
    }

    /// Two shape representations of the definition, both tied to the one
    /// holding the solid: it is listed once from each; a cycle of
    /// relationships ends.
    #[test]
    fn several_roots_and_cycles() {
        let shape = shapes(&format!(
            "{PART}#13=SHAPE_REPRESENTATION('',(#24),#2);#24=AXIS2_PLACEMENT_3D('',#92,$,$);
#15=SHAPE_DEFINITION_REPRESENTATION(#11,#16);#16=SHAPE_REPRESENTATION('',(#24),#2);
#14=SHAPE_REPRESENTATION('',(#20),#2);#20=MANIFOLD_SOLID_BREP('',#90);
#30=SHAPE_REPRESENTATION_RELATIONSHIP('','',#13,#14);
#31=SHAPE_REPRESENTATION_RELATIONSHIP('','',#16,#14);
#32=SHAPE_REPRESENTATION_RELATIONSHIP('','',#14,#16);
#33=SHAPE_REPRESENTATION_RELATIONSHIP('','',#16,#13);"
        ))
        .remove(&10)
        .unwrap();
        assert!(shape.is_complete());
        let roots: Vec<(u64, u64)> = shape
            .solids
            .iter()
            .map(|s| (s.item, s.shape_representation))
            .collect();
        assert_eq!(roots, [(20, 13), (20, 16)]);
    }

    #[test]
    fn links_not_followed() {
        let base = format!(
            "{PART}#13=SHAPE_REPRESENTATION('',(#24),#2);#24=AXIS2_PLACEMENT_3D('',#92,$,$);#14=SHAPE_REPRESENTATION('',(#20),#2);#20=MANIFOLD_SOLID_BREP('',#90);"
        );
        let cases: Vec<(String, (u64, NotFollowedReason))> = vec![
            (
                "#30=(REPRESENTATION_RELATIONSHIP('','',#14,#13)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#31)SHAPE_REPRESENTATION_RELATIONSHIP());#31=ITEM_DEFINED_TRANSFORMATION('','',#20,#24);".to_owned(),
                (30, NotFollowedReason::TransformedRelationship),
            ),
            (
                "#30=REPRESENTATION_RELATIONSHIP('','',#13,#14);".to_owned(),
                (30, NotFollowedReason::NotShapeRelationship),
            ),
            (
                "#30=(REPRESENTATION_RELATIONSHIP('','',#14,#13)SHAPE_REPRESENTATION_RELATIONSHIP()DEFINITIONAL_REPRESENTATION_RELATIONSHIP());".to_owned(),
                (30, NotFollowedReason::NotShapeRelationship),
            ),
            (
                "#30=MECHANICAL_DESIGN_AND_DRAUGHTING_RELATIONSHIP('','',#13,#14);".to_owned(),
                (30, NotFollowedReason::NotShapeRelationship),
            ),
            (
                "#30=SHAPE_REPRESENTATION_RELATIONSHIP('','',#13,#15);#15=SHAPE_REPRESENTATION('',(#20),#3);".to_owned(),
                (30, NotFollowedReason::DifferentContext { contexts: [2, 3] }),
            ),
            (
                "#30=SHAPE_REPRESENTATION_RELATIONSHIP('','',#13,#15);#15=SHAPE_REPRESENTATION_REFERENCE('x',#16);".to_owned(),
                (30, NotFollowedReason::External { document: None }),
            ),
            (
                "#30=SHAPE_REPRESENTATION_RELATIONSHIP('','',#13,#99);".to_owned(),
                (30, NotFollowedReason::Malformed),
            ),
        ];
        for (link, expected) in cases {
            let shape = shapes(&format!("{base}{link}")).remove(&10).unwrap();
            assert_eq!(reasons(&shape), [expected], "{link}");
            assert!(shape.solids.is_empty() && !shape.is_complete());
        }
        let dangling = part("#99");
        assert_eq!(reasons(&dangling), [(99, NotFollowedReason::Malformed)]);
    }

    /// A link into a representation another definition reaches too, other
    /// than as a component placement, is not followed, from either side.
    #[test]
    fn representations_other_definitions_reach() {
        let mut shapes_ = shapes(&format!(
            "{PART}#13=SHAPE_REPRESENTATION('',(#70),#2);
#40=PRODUCT_DEFINITION('q','',$,$);#41=PRODUCT_DEFINITION_SHAPE('','',#40);
#42=SHAPE_DEFINITION_REPRESENTATION(#41,#43);#43=SHAPE_REPRESENTATION('',(#20,#24),#2);
#20=MANIFOLD_SOLID_BREP('',#90);#24=AXIS2_PLACEMENT_3D('',#92,$,$);
#69=REPRESENTATION_MAP(#24,#43);#70=MAPPED_ITEM('',#69,#24);"
        ));
        assert_eq!(
            reasons(&shapes_.remove(&10).unwrap()),
            [(
                70,
                NotFollowedReason::OtherDefinition {
                    definitions: vec![1]
                }
            )]
        );
        assert_eq!(
            reasons(&shapes_.remove(&40).unwrap()),
            [(
                43,
                NotFollowedReason::OtherDefinition {
                    definitions: vec![0]
                }
            )]
        );
    }

    /// Mapped items that map their own representation end the path; a
    /// crafted fan-out of mapped items stops at the walk's budget.
    #[test]
    fn mapped_cycles_and_fan_out() {
        let shape = shapes(&format!(
            "{PART}#13=SHAPE_REPRESENTATION('',(#70),#2);#14=SHAPE_REPRESENTATION('',(#71,#24),#2);
#24=AXIS2_PLACEMENT_3D('',#92,$,$);
#68=REPRESENTATION_MAP(#24,#14);#70=MAPPED_ITEM('',#68,#24);
#69=REPRESENTATION_MAP(#24,#13);#71=MAPPED_ITEM('',#69,#24);"
        ))
        .remove(&10)
        .unwrap();
        assert_eq!(reasons(&shape), [(71, NotFollowedReason::Cycle)]);

        // Eight levels, each mapping the next ten times: 10^8 paths.
        let mut data = format!("{PART}#24=AXIS2_PLACEMENT_3D('',#92,$,$);");
        for level in 0..8u64 {
            let representation = if level == 0 { 13 } else { 100 + level };
            let items: Vec<String> = (0..10)
                .map(|k| format!("#{}", 1000 + level * 10 + k))
                .collect();
            write!(
                data,
                "#{representation}=SHAPE_REPRESENTATION('',({}),#2);#{}=REPRESENTATION_MAP(#24,#{});",
                items.join(","),
                200 + level,
                101 + level
            )
            .unwrap();
            for item in items {
                write!(data, "{item}=MAPPED_ITEM('',#{},#24);", 200 + level).unwrap();
            }
        }
        data += "#108=SHAPE_REPRESENTATION('',(#20),#2);#20=MANIFOLD_SOLID_BREP('',#90);";
        let start = std::time::Instant::now();
        let shape = shapes(&data).remove(&10).unwrap();
        assert!(
            shape
                .not_followed
                .iter()
                .any(|n| n.reason == NotFollowedReason::TooLarge),
            "{:?}",
            shape.not_followed
        );
        assert!(start.elapsed().as_secs() < 10, "{:?}", start.elapsed());
    }

    /// A component stub as `stepq split --master` writes it: a
    /// placement-only representation whose geometry is in another file.
    #[test]
    fn external_definitions_are_not_empty() {
        let shape = shapes(&format!(
            "{PART}#13=SHAPE_REPRESENTATION('',(#24),#2);#24=AXIS2_PLACEMENT_3D('',#92,$,$);
#80=DOCUMENT_FILE('plate.stp','',$,#81,'',$);
#82=APPLIED_DOCUMENT_REFERENCE(#80,'',(#10));
#83=PROPERTY_DEFINITION('external definition',$,#80);
#84=PROPERTY_DEFINITION_REPRESENTATION(#83,#13);"
        ))
        .remove(&10)
        .unwrap();
        assert_eq!(
            reasons(&shape),
            [
                (82, NotFollowedReason::External { document: Some(80) }),
                (84, NotFollowedReason::External { document: Some(80) }),
            ]
        );
        assert!(shape.solids.is_empty() && !shape.is_complete());
    }

    /// The lists of relationship and mapped-item types are exactly
    /// `representation_relationship` and `mapped_item` and their subtypes
    /// in the four schemas (`tools/fetch-schemas.sh`; skipped without them,
    /// failed with `STEPQ_REQUIRE_FIXTURES=1`, as in CI).
    #[test]
    fn relationship_and_mapped_item_types_match_the_schemas() {
        use crate::express::Schema;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/schemas");
        let mut found: [HashSet<String>; 2] = [HashSet::new(), HashSet::new()];
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
            for (root, found) in ["REPRESENTATION_RELATIONSHIP", "MAPPED_ITEM"]
                .iter()
                .zip(&mut found)
            {
                // Walked per schema: a subtype may be in one schema only.
                let mut subtypes = vec![(*root).to_owned()];
                let mut seen = HashSet::from([(*root).to_owned()]);
                while let Some(name) = subtypes.pop() {
                    for entity in schema.entities() {
                        if entity
                            .supertypes
                            .iter()
                            .any(|s| s.eq_ignore_ascii_case(&name))
                            && seen.insert(entity.name.clone())
                        {
                            subtypes.push(entity.name.clone());
                        }
                    }
                }
                found.extend(seen);
            }
        }
        let required =
            std::env::var_os("STEPQ_REQUIRE_FIXTURES").is_some_and(|v| v != "0" && !v.is_empty());
        if read < 4 {
            assert!(
                !required,
                "{read} of 4 schemas under {} (run tools/fetch-schemas.sh)",
                dir.display()
            );
            eprintln!("skipping: {read} of 4 schemas (run tools/fetch-schemas.sh)");
            return;
        }
        for (list, found) in [RELATIONSHIPS, MAPPED_ITEMS].iter().zip(&found) {
            let listed: HashSet<String> = list.iter().map(|&name| name.to_owned()).collect();
            assert_eq!(&listed, found);
        }
    }

    /// What the review of this walk found it missed or misread.
    #[test]
    fn documents_mapped_item_subtypes_and_odd_relationships() {
        // An AP203 definition with its geometry in an associated document.
        let mut shapes_ = shapes(
            "#10=PRODUCT_DEFINITION_WITH_ASSOCIATED_DOCUMENTS('p','',$,$,(#80));
#11=PRODUCT_DEFINITION_SHAPE('','',#10);#12=SHAPE_DEFINITION_REPRESENTATION(#11,#13);
#13=SHAPE_REPRESENTATION('',(#24),#2);#24=AXIS2_PLACEMENT_3D('',#92,$,$);
#80=DOCUMENT_FILE('plate.stp','',$,#81,'',$);
#50=PRODUCT_DEFINITION('a','',$,$);#60=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','','',#50,#10,$);",
        );
        assert_eq!(
            reasons(&shapes_.remove(&10).unwrap()),
            [(80, NotFollowedReason::External { document: Some(80) })]
        );

        // A camera image maps a representation holding a solid.
        let camera = part("#70").not_followed.len();
        assert_eq!(camera, 1, "an undefined #70");
        let shape = shapes(&format!(
            "{PART}#13=SHAPE_REPRESENTATION('',(#70),#2);#14=SHAPE_REPRESENTATION('',(#20,#24),#2);
#20=MANIFOLD_SOLID_BREP('',#90);#24=AXIS2_PLACEMENT_3D('',#92,$,$);#69=REPRESENTATION_MAP(#24,#14);
#70=(CAMERA_IMAGE()CAMERA_IMAGE_3D_WITH_SCALE()GEOMETRIC_REPRESENTATION_ITEM()MAPPED_ITEM(#69,#24)REPRESENTATION_ITEM(''));"
        ))
        .remove(&10)
        .unwrap();
        assert_eq!(
            reasons(&shape),
            [(70, NotFollowedReason::NotPlainMappedItem)]
        );
        assert_eq!(shape.other, []);

        // A relationship whose rep_1 and rep_2 are not attributes 2 and 3.
        let shape = shapes(&format!(
            "{PART}#13=SHAPE_REPRESENTATION('',(#24),#2);#24=AXIS2_PLACEMENT_3D('',#92,$,$);
#14=SHAPE_REPRESENTATION('',(#20),#2);#20=MANIFOLD_SOLID_BREP('',#90);
#30=EVALUATED_CHARACTERISTIC('',(),#2,'','',#13,#14);"
        ))
        .remove(&10)
        .unwrap();
        assert_eq!(
            reasons(&shape),
            [(30, NotFollowedReason::NotShapeRelationship)]
        );
    }

    /// A component's mapped item is skipped by its assembly only; any other
    /// definition listing it places the component's shape, which is not its
    /// own.
    #[test]
    fn component_mapped_items_belong_to_their_assembly() {
        let mut shapes_ = shapes(&format!(
            "{PART}#13=SHAPE_REPRESENTATION('',(#20,#24),#2);#20=MANIFOLD_SOLID_BREP('',#90);
#24=AXIS2_PLACEMENT_3D('',#92,$,$);
#50=PRODUCT_DEFINITION('a','',$,$);#51=PRODUCT_DEFINITION_SHAPE('','',#50);
#52=SHAPE_DEFINITION_REPRESENTATION(#51,#53);#53=SHAPE_REPRESENTATION('',(#24,#70),#2);
#60=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','','',#50,#10,$);
#69=REPRESENTATION_MAP(#24,#13);#70=MAPPED_ITEM('',#69,#24);
#40=PRODUCT_DEFINITION('b','',$,$);#41=PRODUCT_DEFINITION_SHAPE('','',#40);
#42=SHAPE_DEFINITION_REPRESENTATION(#41,#43);#43=SHAPE_REPRESENTATION('',(#70),#2);"
        ));
        assert_eq!(shapes_.remove(&50).unwrap(), DefinitionShape::default());
        assert_eq!(
            reasons(&shapes_.remove(&40).unwrap()),
            [(
                70,
                NotFollowedReason::OtherDefinition {
                    definitions: vec![0]
                }
            )]
        );
    }

    /// Many definitions sharing one representation each list at most eight
    /// of the others.
    #[test]
    fn owners_are_listed_up_to_eight() {
        let mut data =
            "#13=SHAPE_REPRESENTATION('',(#20),#2);#20=MANIFOLD_SOLID_BREP('',#90);".to_owned();
        for d in 0..20u64 {
            let base = 100 + d * 3;
            write!(
                data,
                "#{base}=PRODUCT_DEFINITION('{d}','',$,$);#{}=PRODUCT_DEFINITION_SHAPE('','',#{base});                 #{}=SHAPE_DEFINITION_REPRESENTATION(#{},#13);",
                base + 1,
                base + 2,
                base + 1
            )
            .unwrap();
        }
        for shape in shapes(&data).values() {
            let reasons = reasons(shape);
            let [(13, NotFollowedReason::OtherDefinition { definitions })] = reasons.as_slice()
            else {
                panic!("{shape:?}");
            };
            assert!((7..=8).contains(&definitions.len()), "{definitions:?}");
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn shapes_have_unique_keys() {
        use crate::testing::unique;
        let shape = DefinitionShape {
            solids: vec![ShapeItem {
                item: 1,
                entity: "MANIFOLD_SOLID_BREP".to_owned(),
                representation: 2,
                context: 3,
                shape_representation: 4,
                path: vec![
                    ShapeLink::Relationship {
                        relationship: 5,
                        representation: 2,
                    },
                    ShapeLink::MappedItem {
                        mapped_item: 6,
                        representation_map: 7,
                        origin: 8,
                        target: 9,
                        representation: 2,
                    },
                ],
            }],
            other: Vec::new(),
            not_followed: [
                NotFollowedReason::External { document: Some(1) },
                NotFollowedReason::TransformedRelationship,
                NotFollowedReason::NotShapeRelationship,
                NotFollowedReason::NotPlainMappedItem,
                NotFollowedReason::DifferentContext { contexts: [1, 2] },
                NotFollowedReason::OtherDefinition {
                    definitions: vec![1],
                },
                NotFollowedReason::UnexpectedTarget { expected: "x" },
                NotFollowedReason::Cycle,
                NotFollowedReason::TooLarge,
                NotFollowedReason::Malformed,
            ]
            .into_iter()
            .map(|reason| NotFollowed {
                instance: 1,
                entity: "X".to_owned(),
                reason,
            })
            .collect(),
        };
        let json = unique(&shape);
        assert!(json.contains("\"kind\":\"mapped_item\""), "{json}");
        assert!(json.contains("\"kind\":\"different_context\""), "{json}");
    }
}
