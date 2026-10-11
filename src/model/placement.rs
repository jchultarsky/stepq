//! Placements: the transform each assembly usage states, in millimetres,
//! and every occurrence of a definition under a root. See [`Placements`].

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::iter::FusedIterator;

use super::assembly::context_dependent_shape_representations;
use super::{Graph, Placement, ProductStructure, Usage, UsageKind};
use crate::geometry::{Axis2Placement3d, GeometryError, GeometryErrorKind, Reader, Rigid};
use crate::p21::{Exchange, Param};
use crate::policy::MAX_ASSEMBLY_DEPTH;
use crate::record;
use crate::units::UnitError;

const RRWT: &str = "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION";

// Why a usage is inconsistent; see `PlacementError::Inconsistent`.
const MORE_THAN_ONE_LINK: &str =
    "the usage has more than one context_dependent_shape_representation";
const REPEATED: &str = "a partial entity is repeated, so its attributes are stated twice";
const UNDEFINED: &str = "refers to an instance that is not defined";
const RELATIONSHIP_LAYOUT: &str =
    "not a representation_relationship with the attributes the schema gives it";
const NOT_A_REFERENCE: &str = "rep_1, rep_2 or transformation_operator is not a reference";
const SIDES: &str = "rep_1 and rep_2 are not one of the component's shape representations and \
     one of the assembly's";
const TRANSFORMATION_LAYOUT: &str =
    "not an item_defined_transformation with two transform items, as the schema gives it";
const NOT_A_REPRESENTATION: &str = "cannot be read as a representation";
const ITEM_1: &str = "transform_item_1 is not an item of rep_1";
const ITEM_2: &str = "transform_item_2 is not an item of rep_2";
const MAPPED_ITEM_LAYOUT: &str =
    "not a mapped_item with a mapping source and target, as the schema gives it";
const NOT_HELD: &str = "no shape representation of the assembly lists the mapped item";
const HELD_TWICE: &str = "more than one shape representation of the assembly lists the mapped \
     item, so the context of its target is not certain";
const SHARED_MAPPED_ITEM: &str = "the mapped item places more than one usage, so which one \
     it places is not certain";
const MAP_LAYOUT: &str =
    "not a representation_map with an origin and a representation, as the schema gives it";
const NOT_THE_COMPONENT: &str = "maps no shape representation of the component";
const CHANGES_REPRESENTATION: &str = "the assembly is placed through one of its shape \
     representations and holds this component in another";
const AMBIGUOUS_ROOT: &str =
    "the root's usages place its components in different shape representations of it";

/// The transform of every assembly usage of a [`ProductStructure`], read
/// from the entities its [`Placement`]s identify, and the occurrences they
/// compose to.
///
/// Built once per file by [`new`](Self::new), which never fails as a whole:
/// each usage has its own result. Transforms are in millimetres and map
/// component coordinates into assembly coordinates ([`Rigid`]).
///
/// [`ProductStructure`] identifies the entities that place each usage
/// ([`Placement`]); this reads them. Each frame is read through a
/// [`geometry::Reader`](crate::geometry::Reader) in the context its
/// position assigns (ISO 10303-43): `transform_item_k` in `rep_k`'s, a
/// `mapping_origin` in its `mapped_representation`'s, and a
/// `mapping_target` in the context of the representation that lists the
/// mapped item. Transforms are composed in millimetres, from the root down.
///
/// * An `item_defined_transformation` carries the component's frame onto the
///   assembly's: `M(assembly-side item) · M(component-side item)⁻¹`. Its
///   `transform_item_k` is an item of `rep_k` (AP242's WR2 of
///   `representation_relationship_with_transformation`), and the usage, not
///   the relationship's order, decides which of `rep_1` and `rep_2` is the
///   component's: real files reverse them.
/// * A `mapped_item` maps its `mapping_origin` onto its `mapping_target`:
///   `M(target) · M(origin)⁻¹`.
///
/// What cannot be read as stated is a [`PlacementError`] for that usage,
/// and for every occurrence below it; the rest of the file still reads.
/// See `docs/ARCHITECTURE.md`, "Assembly structure" and "Reading stated
/// geometry".
///
/// ```
/// use stepq::model::{Graph, Placements, ProductStructure};
/// use stepq::p21::parse;
///
/// // Assembly #10 holds part #30, turned a quarter about z and moved
/// // 10 mm along x.
/// let src = b"ISO-10303-21;HEADER;FILE_SCHEMA(('AP214'));ENDSEC;DATA;
/// #1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
/// #2=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#1))
///   REPRESENTATION_CONTEXT('',''));
/// #3=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#1))
///   REPRESENTATION_CONTEXT('',''));
/// #10=PRODUCT_DEFINITION('assembly','',$,$);
/// #11=PRODUCT_DEFINITION_SHAPE('','',#10);
/// #12=SHAPE_DEFINITION_REPRESENTATION(#11,#13);
/// #13=SHAPE_REPRESENTATION('',(#40),#2);
/// #30=PRODUCT_DEFINITION('part','',$,$);
/// #31=PRODUCT_DEFINITION_SHAPE('','',#30);
/// #32=SHAPE_DEFINITION_REPRESENTATION(#31,#33);
/// #33=SHAPE_REPRESENTATION('',(#50),#3);
/// #40=AXIS2_PLACEMENT_3D('',#41,#42,#43);
/// #41=CARTESIAN_POINT('',(10.,0.,0.));
/// #42=DIRECTION('',(0.,0.,1.));
/// #43=DIRECTION('',(0.,1.,0.));
/// #50=AXIS2_PLACEMENT_3D('',#51,$,$);
/// #51=CARTESIAN_POINT('',(0.,0.,0.));
/// #60=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','','',#10,#30,$);
/// #61=PRODUCT_DEFINITION_SHAPE('','',#60);
/// #62=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#63,#61);
/// #63=(REPRESENTATION_RELATIONSHIP('','',#33,#13)
///   REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#64)
///   SHAPE_REPRESENTATION_RELATIONSHIP());
/// #64=ITEM_DEFINED_TRANSFORMATION('','',#50,#40);
/// ENDSEC;END-ISO-10303-21;";
///
/// let graph = Graph::new(parse(src)?)?;
/// let structure = ProductStructure::new(&graph);
/// let placements = Placements::new(&graph, &structure);
/// let part = placements.usage(0)?;
/// assert_eq!(part.apply_point([1.0, 0.0, 0.0]), [10.0, 1.0, 0.0]);
///
/// let occurrences: Vec<_> = placements.occurrences(structure.roots()[0]).collect();
/// assert_eq!(occurrences.len(), 2); // the assembly, then the part
/// assert_eq!(occurrences[1].path, [0]);
/// assert_eq!(occurrences[1].representation, Some(33));
/// assert_eq!(occurrences[1].transform, Ok(part));
/// # Ok::<(), stepq::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct Placements<'s> {
    structure: &'s ProductStructure,
    usages: Vec<UsageRead>,
}

/// What [`Placements::new`] read for one usage.
#[derive(Debug, Clone)]
struct UsageRead {
    /// The component-side and assembly-side shape representations, as soon
    /// as the placement names them, whether or not its transform reads.
    sides: Option<Sides>,
    result: Result<Placed, PlacementError>,
}

#[derive(Debug, Clone, Copy)]
struct Sides {
    component: u64,
    assembly: u64,
}

#[derive(Debug, Clone)]
struct Placed {
    transform: Rigid,
    /// Reported when a transform composed with this one is not finite: the
    /// usage, at its assembly-side frame.
    overflow: PlacementError,
}

impl<'s> Placements<'s> {
    /// Reads the placement of every usage of `structure`, which must have
    /// been read from `graph` (from another graph, the results mean
    /// nothing).
    ///
    /// This never fails as a whole: each usage has its own result, read
    /// with [`usage`](Self::usage).
    pub fn new(graph: &Graph<'_>, structure: &'s ProductStructure) -> Self {
        let mut frames = Frames {
            graph,
            readers: HashMap::new(),
            items: HashMap::new(),
        };
        let mut usages: Vec<UsageRead> = structure
            .usages()
            .iter()
            .map(|usage| read_usage(&mut frames, structure, usage))
            .collect();
        refuse_shared_mapped_items(structure, &mut usages);
        Self { structure, usages }
    }

    /// The transform of usage `usage` (an index into
    /// [`ProductStructure::usages`]): component coordinates (mm) to assembly
    /// coordinates (mm).
    ///
    /// # Errors
    ///
    /// Why the usage cannot be placed as stated; see [`PlacementError`].
    /// [`Cycle`](PlacementError::Cycle) and
    /// [`TooDeep`](PlacementError::TooDeep) belong to occurrences, not to
    /// usages, and are not returned here.
    ///
    /// # Panics
    ///
    /// Panics if `usage` is out of range.
    pub fn usage(&self, usage: usize) -> Result<Rigid, PlacementError> {
        self.usages[usage]
            .result
            .as_ref()
            .map(|placed| placed.transform)
            .map_err(Clone::clone)
    }

    /// Every occurrence under `root` (an index into
    /// [`ProductStructure::definitions`], usually one of its
    /// [`roots`](ProductStructure::roots)): the root first, with an empty
    /// path and the identity, then depth first in file order. A
    /// sub-assembly used n times appears n times, once per path; separate
    /// roots are unrelated frames.
    ///
    /// A usage that places a definition already on its path ends the path
    /// with [`Cycle`](PlacementError::Cycle), and one more than 1,024 levels
    /// below the root with [`TooDeep`](PlacementError::TooDeep); their
    /// components are not listed. The iterator is lazy and holds only the
    /// current path, so its memory is proportional to depth. The number of
    /// occurrences is not bounded: a crafted file can reach the same
    /// sub-assembly by exponentially many paths, so a caller that collects
    /// them sets its own limit.
    ///
    /// # Panics
    ///
    /// Panics if `root` is out of range.
    pub fn occurrences(&self, root: usize) -> Occurrences<'_> {
        let space = self.root_space(root);
        Occurrences {
            placements: self,
            root: Some(Occurrence {
                path: Vec::new(),
                definition: root,
                representation: space.representation(),
                transform: Ok(Rigid::IDENTITY),
            }),
            stack: vec![Level {
                definition: root,
                transform: Ok(Rigid::IDENTITY),
                space,
                next: 0,
            }],
            path: Vec::new(),
            on_path: HashSet::from([root]),
        }
    }

    /// The shape representation a root holds its components in: the one its
    /// usages name, or, if none names one, its only shape representation.
    fn root_space(&self, root: usize) -> Space {
        let mut named: Option<u64> = None;
        for &index in self.structure.child_usages(root) {
            let Some(sides) = self.usages[index].sides else {
                continue;
            };
            match named {
                Some(representation) if representation != sides.assembly => {
                    return Space::Ambiguous;
                }
                _ => named = Some(sides.assembly),
            }
        }
        match (
            named,
            self.structure.definitions()[root]
                .shape_representations
                .as_slice(),
        ) {
            (Some(representation), _) | (None, &[representation]) => Space::Of(representation),
            _ => Space::Unknown,
        }
    }
}

/// One place a definition occurs in an assembly tree, from
/// [`Placements::occurrences`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Occurrence {
    /// The usages from the root down, as indices into
    /// [`ProductStructure::usages`]; empty for the root. Each usage's
    /// `instance` is the `#id` that addresses it in the file.
    pub path: Vec<usize>,
    /// The definition that occurs here, an index into
    /// [`ProductStructure::definitions`].
    pub definition: usize,
    /// The space `transform` maps from: the component-side shape
    /// representation of the last usage, or for the root the one its usages
    /// share (its only shape representation if they name none). `None` if
    /// the last usage names none, or for the root if its usages name several
    /// or, naming none, it has several shape representations or none. The
    /// representations of the root and of sub-assemblies may hold geometry
    /// of their own besides their components.
    pub representation: Option<u64>,
    /// This occurrence's coordinates (mm) to the root's (mm), or why they
    /// cannot be read: [`Cycle`](PlacementError::Cycle) or
    /// [`TooDeep`](PlacementError::TooDeep) if the path ends here, otherwise
    /// the first refusal on the path from the root.
    #[cfg_attr(feature = "serde", serde(serialize_with = "outcome"))]
    pub transform: Result<Rigid, PlacementError>,
}

/// Serializes a transform as `{"ok": transform}` or `{"error": error}`.
#[cfg(feature = "serde")]
fn outcome<S: serde::Serializer>(
    result: &Result<Rigid, PlacementError>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(Some(1))?;
    match result {
        Ok(transform) => map.serialize_entry("ok", transform)?,
        Err(error) => map.serialize_entry("error", error)?,
    }
    map.end()
}

/// The occurrences under a root, depth first; see
/// [`Placements::occurrences`].
#[derive(Debug, Clone)]
pub struct Occurrences<'p> {
    placements: &'p Placements<'p>,
    /// The root, until it is returned.
    root: Option<Occurrence>,
    /// The definitions on the current path, the root first.
    stack: Vec<Level>,
    /// The usages on the current path.
    path: Vec<usize>,
    /// The definitions in `stack`.
    on_path: HashSet<usize>,
}

#[derive(Debug, Clone)]
struct Level {
    definition: usize,
    transform: Result<Rigid, PlacementError>,
    /// The representation this definition holds its components in.
    space: Space,
    /// The next of its usages to visit.
    next: usize,
}

/// The shape representation an assembly occurrence is placed through.
#[derive(Debug, Clone, Copy)]
enum Space {
    Of(u64),
    /// Not named: the usage that reaches it is not placed.
    Unknown,
    /// A root whose usages name several.
    Ambiguous,
}

impl Space {
    fn representation(self) -> Option<u64> {
        match self {
            Self::Of(representation) => Some(representation),
            Self::Unknown | Self::Ambiguous => None,
        }
    }
}

impl Iterator for Occurrences<'_> {
    type Item = Occurrence;

    fn next(&mut self) -> Option<Occurrence> {
        if let Some(root) = self.root.take() {
            return Some(root);
        }
        let Self {
            placements,
            stack,
            path,
            on_path,
            ..
        } = self;
        loop {
            let top = stack.last_mut()?;
            let Some(&index) = placements
                .structure
                .child_usages(top.definition)
                .get(top.next)
            else {
                let done = stack.pop()?;
                on_path.remove(&done.definition);
                if !stack.is_empty() {
                    path.pop();
                }
                continue;
            };
            top.next += 1;
            let usage = &placements.structure.usages()[index];
            let read = &placements.usages[index];
            path.push(index);
            let (transform, descend) = if on_path.contains(&usage.child) {
                (
                    Err(PlacementError::Cycle {
                        usage: usage.instance,
                    }),
                    false,
                )
            } else if path.len() > MAX_ASSEMBLY_DEPTH {
                (
                    Err(PlacementError::TooDeep {
                        usage: usage.instance,
                    }),
                    false,
                )
            } else {
                (compose(&top.transform, top.space, usage, read), true)
            };
            let representation = read.sides.map(|sides| sides.component);
            let occurrence = Occurrence {
                path: path.clone(),
                definition: usage.child,
                representation,
                transform: transform.clone(),
            };
            if descend {
                on_path.insert(usage.child);
                stack.push(Level {
                    definition: usage.child,
                    transform,
                    space: representation.map_or(Space::Unknown, Space::Of),
                    next: 0,
                });
            } else {
                path.pop();
            }
            return Some(occurrence);
        }
    }
}

impl FusedIterator for Occurrences<'_> {}

/// The transform of the component `usage` places, under an assembly
/// occurrence at `parent` that holds its components in `space`.
fn compose(
    parent: &Result<Rigid, PlacementError>,
    space: Space,
    usage: &Usage,
    read: &UsageRead,
) -> Result<Rigid, PlacementError> {
    let parent = parent.as_ref().map_err(Clone::clone)?;
    let placed = read.result.as_ref().map_err(Clone::clone)?;
    if let Some(sides) = read.sides {
        let reason = match space {
            Space::Of(representation) if representation != sides.assembly => {
                Some(CHANGES_REPRESENTATION)
            }
            Space::Ambiguous => Some(AMBIGUOUS_ROOT),
            Space::Of(_) | Space::Unknown => None,
        };
        if let Some(reason) = reason {
            return Err(PlacementError::Inconsistent {
                usage: usage.instance,
                instance: sides.assembly,
                reason,
            });
        }
    }
    let transform = *parent * placed.transform;
    if is_finite(&transform) {
        Ok(transform)
    } else {
        Err(placed.overflow.clone())
    }
}

pub(super) fn is_finite(transform: &Rigid) -> bool {
    transform
        .rotation
        .iter()
        .flatten()
        .chain(&transform.translation)
        .all(|value| value.is_finite())
}

/// Readers and item sets per representation, each built once: a file can
/// hold thousands of usages placed through one representation.
struct Frames<'g, 'a> {
    graph: &'g Graph<'a>,
    readers: HashMap<u64, Result<Reader<'g, 'a>, UnitError>>,
    /// The items of each representation, `None` if it cannot be read as one.
    items: HashMap<u64, Option<HashSet<u64>>>,
}

impl Frames<'_, '_> {
    /// The items of `#representation`, or `None` if it cannot be read as a
    /// representation.
    fn items(&mut self, representation: u64) -> Option<&HashSet<u64>> {
        let exchange = self.graph.exchange();
        self.items
            .entry(representation)
            .or_insert_with(|| {
                record::representation(exchange, representation)
                    .ok()
                    .map(|read| read.items.into_iter().collect())
            })
            .as_ref()
    }

    /// The `axis2_placement_3d` `#item`, read in the context of
    /// `#representation`, for usage `#usage`.
    fn read(
        &mut self,
        usage: u64,
        item: u64,
        representation: u64,
    ) -> Result<Axis2Placement3d, PlacementError> {
        let graph = self.graph;
        let exchange = graph.exchange();
        if let Some(instance) = exchange.get(item) {
            let is_frame = !instance.is_complex()
                && exchange
                    .records(instance)
                    .next()
                    .is_some_and(|record| record.is("AXIS2_PLACEMENT_3D"));
            if !is_frame {
                return Err(unsupported(exchange, usage, item));
            }
        }
        let reader = self
            .readers
            .entry(representation)
            .or_insert_with(|| Reader::for_representation(graph, representation));
        let reader = match reader {
            Ok(reader) => reader,
            Err(error) => {
                return Err(PlacementError::Frame {
                    usage,
                    error: GeometryError {
                        instance: item,
                        entity: entity(exchange, item),
                        attribute: None,
                        kind: GeometryErrorKind::Unit(error.clone()),
                    },
                });
            }
        };
        let frame = reader
            .axis2_placement_3d(item)
            .map_err(|error| PlacementError::Frame { usage, error })?;
        #[expect(
            clippy::float_cmp,
            reason = "Open CASCADE and the schema build the same x only for an axis of exactly +Z"
        )]
        let plus_z = frame.z == [0.0, 0.0, 1.0];
        if !frame.ref_direction_stated && !plus_z {
            return Err(PlacementError::Divergent {
                usage,
                instance: item,
                reason: DivergentReason::DefaultedRefDirection,
            });
        }
        Ok(frame)
    }
}

fn read_usage(
    frames: &mut Frames<'_, '_>,
    structure: &ProductStructure,
    usage: &Usage,
) -> UsageRead {
    let definitions = structure.definitions();
    let mut reading = Reading {
        frames,
        usage: usage.instance,
        component_shapes: &definitions[usage.child].shape_representations,
        assembly_shapes: &definitions[usage.parent].shape_representations,
        sides: None,
    };
    let result = match &usage.placement {
        None => Err(PlacementError::Unplaced {
            usage: usage.instance,
        }),
        Some(Placement::ShapeRelationship { relationship, .. }) => {
            reading.shape_relationship(*relationship)
        }
        Some(Placement::MappedItem { mapped_item, .. }) => reading.mapped_item(*mapped_item),
    };
    let sides = reading.sides;
    UsageRead {
        sides,
        result: match quantity_refusal(usage) {
            Some(refusal) => Err(refusal),
            None => result,
        },
    }
}

/// A mapped item places one occurrence, so one that two usages claim (an
/// assembly whose shape is defined twice, or a representation two components
/// share) places neither for certain: each such usage is refused.
fn refuse_shared_mapped_items(structure: &ProductStructure, usages: &mut [UsageRead]) {
    let mapped_item = |usage: &Usage| match usage.placement {
        Some(Placement::MappedItem { mapped_item, .. }) => Some(mapped_item),
        _ => None,
    };
    let mut claims: HashMap<u64, usize> = HashMap::new();
    for item in structure.usages().iter().filter_map(mapped_item) {
        *claims.entry(item).or_default() += 1;
    }
    for (usage, read) in structure.usages().iter().zip(usages) {
        let Some(item) = mapped_item(usage) else {
            continue;
        };
        if claims.get(&item).is_some_and(|&count| count > 1) && read.result.is_ok() {
            read.result = Err(PlacementError::Inconsistent {
                usage: usage.instance,
                instance: item,
                reason: SHARED_MAPPED_ITEM,
            });
        }
    }
}

/// A quantified usage stands for `quantity` occurrences, which one placement
/// cannot place unless it is exactly one.
fn quantity_refusal(usage: &Usage) -> Option<PlacementError> {
    if usage.kind != UsageKind::QuantifiedAssemblyComponentUsage {
        return None;
    }
    // Only a quantity of exactly one is one occurrence.
    let one = usage.quantity == Some(1.0);
    (!one).then_some(PlacementError::Quantified {
        usage: usage.instance,
        quantity: usage.quantity,
    })
}

/// The reading of one usage's placement.
struct Reading<'r, 'g, 'a> {
    frames: &'r mut Frames<'g, 'a>,
    usage: u64,
    component_shapes: &'r [u64],
    assembly_shapes: &'r [u64],
    sides: Option<Sides>,
}

/// What a representation relationship states.
struct Relationship {
    rep_1: u64,
    rep_2: u64,
    transformation: Option<u64>,
}

impl<'g, 'a> Reading<'_, 'g, 'a> {
    fn exchange(&self) -> &'g Exchange<'a> {
        self.frames.graph.exchange()
    }

    fn inconsistent(&self, instance: u64, reason: &'static str) -> PlacementError {
        PlacementError::Inconsistent {
            usage: self.usage,
            instance,
            reason,
        }
    }

    /// A usage tied by a `context_dependent_shape_representation` to
    /// `#relationship`.
    fn shape_relationship(&mut self, relationship: u64) -> Result<Placed, PlacementError> {
        let graph = self.frames.graph;
        let node = graph
            .node(self.usage)
            .ok_or_else(|| self.inconsistent(self.usage, UNDEFINED))?;
        if let [_, second, ..] = context_dependent_shape_representations(graph, node).as_slice() {
            return Err(self.inconsistent(*second, MORE_THAN_ONE_LINK));
        }
        let Relationship {
            rep_1,
            rep_2,
            transformation,
        } = self.relationship(relationship)?;
        let normal =
            self.component_shapes.contains(&rep_1) && self.assembly_shapes.contains(&rep_2);
        let reversed =
            self.component_shapes.contains(&rep_2) && self.assembly_shapes.contains(&rep_1);
        if normal == reversed {
            return Err(self.inconsistent(relationship, SIDES));
        }
        self.sides = Some(if normal {
            Sides {
                component: rep_1,
                assembly: rep_2,
            }
        } else {
            Sides {
                component: rep_2,
                assembly: rep_1,
            }
        });
        let transformation = transformation.ok_or(PlacementError::NoTransformation {
            usage: self.usage,
            relationship,
        })?;
        let [item_1, item_2] = self.item_defined_transformation(relationship, transformation)?;
        self.listed(item_1, rep_1, ITEM_1)?;
        self.listed(item_2, rep_2, ITEM_2)?;
        let frame_1 = self.frames.read(self.usage, item_1, rep_1)?;
        let frame_2 = self.frames.read(self.usage, item_2, rep_2)?;
        if !frame_1.to_rigid().is_identity() && !frame_2.to_rigid().is_identity() {
            return Err(PlacementError::Divergent {
                usage: self.usage,
                instance: transformation,
                reason: DivergentReason::BothFramesPlaced,
            });
        }
        if normal {
            self.placed(&frame_2, &frame_1)
        } else {
            self.placed(&frame_1, &frame_2)
        }
    }

    /// Reads `#id` as a representation relationship, simple or complex, with
    /// the attribute counts the schema gives it.
    fn relationship(&self, id: u64) -> Result<Relationship, PlacementError> {
        let exchange = self.exchange();
        let instance = exchange
            .get(id)
            .ok_or_else(|| self.inconsistent(id, UNDEFINED))?;
        if record::repeats_partial(exchange, instance) {
            return Err(self.inconsistent(id, REPEATED));
        }
        let layout = || self.inconsistent(id, RELATIONSHIP_LAYOUT);
        let (base, transformation): (Vec<Param<'_>>, Option<Param<'_>>) = if instance.is_complex() {
            let base = record::partial(exchange, instance, "REPRESENTATION_RELATIONSHIP")
                .and_then(|r| record::exact_params(r, 4))
                .ok_or_else(layout)?;
            let transformation = match record::partial(exchange, instance, RRWT) {
                None => None,
                Some(r) => Some(
                    record::exact_params(r, 1)
                        .and_then(|params| params.into_iter().next())
                        .ok_or_else(layout)?,
                ),
            };
            (base, transformation)
        } else {
            let record = exchange.records(instance).next().ok_or_else(layout)?;
            if record.is(RRWT) {
                let mut params = record::exact_params(record, 5).ok_or_else(layout)?;
                let transformation = params.pop();
                (params, transformation)
            } else if record::upper(record.name()).ends_with("REPRESENTATION_RELATIONSHIP") {
                (record::exact_params(record, 4).ok_or_else(layout)?, None)
            } else {
                return Err(unsupported(exchange, self.usage, id));
            }
        };
        let reference = |param: &Param<'_>| {
            param
                .reference()
                .ok_or_else(|| self.inconsistent(id, NOT_A_REFERENCE))
        };
        Ok(Relationship {
            rep_1: reference(&base[2])?,
            rep_2: reference(&base[3])?,
            transformation: transformation.as_ref().map(reference).transpose()?,
        })
    }

    /// The two items of the `item_defined_transformation` `#id`, which
    /// `#relationship` names.
    fn item_defined_transformation(
        &self,
        relationship: u64,
        id: u64,
    ) -> Result<[u64; 2], PlacementError> {
        let exchange = self.exchange();
        let instance = exchange
            .get(id)
            .ok_or_else(|| self.inconsistent(relationship, UNDEFINED))?;
        let record = if instance.is_complex() {
            None
        } else {
            exchange
                .records(instance)
                .next()
                .filter(|record| record.is("ITEM_DEFINED_TRANSFORMATION"))
        };
        let Some(record) = record else {
            return Err(unsupported(exchange, self.usage, id));
        };
        let params = record::exact_params(record, 4)
            .ok_or_else(|| self.inconsistent(id, TRANSFORMATION_LAYOUT))?;
        let item = |param: &Param<'_>| {
            param
                .reference()
                .ok_or_else(|| self.inconsistent(id, TRANSFORMATION_LAYOUT))
        };
        Ok([item(&params[2])?, item(&params[3])?])
    }

    /// Requires `#item` to be listed among the items of `#representation`.
    fn listed(
        &mut self,
        item: u64,
        representation: u64,
        reason: &'static str,
    ) -> Result<(), PlacementError> {
        match self.frames.items(representation) {
            None => Err(self.inconsistent(representation, NOT_A_REPRESENTATION)),
            Some(items) if items.contains(&item) => Ok(()),
            Some(_) => Err(self.inconsistent(item, reason)),
        }
    }

    /// A usage placed by the mapped item `#item` in the assembly's shape.
    fn mapped_item(&mut self, item: u64) -> Result<Placed, PlacementError> {
        let exchange = self.exchange();
        let instance = exchange
            .get(item)
            .ok_or_else(|| self.inconsistent(item, UNDEFINED))?;
        if record::repeats_partial(exchange, instance) {
            return Err(self.inconsistent(item, REPEATED));
        }
        let params = if instance.is_complex() {
            // A complex mapped item that is anything more than a placed
            // representation item (Creo's scaled camera images, say) is not
            // read as a placement.
            let other = exchange.records(instance).any(|record| {
                ![
                    "MAPPED_ITEM",
                    "REPRESENTATION_ITEM",
                    "GEOMETRIC_REPRESENTATION_ITEM",
                ]
                .iter()
                .any(|name| record.is(name))
            });
            if other {
                return Err(unsupported(exchange, self.usage, item));
            }
            record::partial(exchange, instance, "MAPPED_ITEM")
                .and_then(|record| record::exact_params(record, 2))
        } else {
            exchange
                .records(instance)
                .next()
                .filter(|record| record.is("MAPPED_ITEM"))
                .and_then(|record| record::exact_params(record, 3))
                .map(|params| params[1..].to_vec())
        }
        .ok_or_else(|| self.inconsistent(item, MAPPED_ITEM_LAYOUT))?;
        let (Some(map), Some(target)) = (params[0].reference(), params[1].reference()) else {
            return Err(self.inconsistent(item, MAPPED_ITEM_LAYOUT));
        };

        let mut holders: Vec<u64> = Vec::new();
        for &representation in self.assembly_shapes {
            if self
                .frames
                .items(representation)
                .is_some_and(|items| items.contains(&item))
            {
                holders.push(representation);
            }
        }
        holders.sort_unstable();
        holders.dedup();
        let holder = match holders.as_slice() {
            [holder] => *holder,
            [] => return Err(self.inconsistent(item, NOT_HELD)),
            _ => return Err(self.inconsistent(item, HELD_TWICE)),
        };

        let map_instance = exchange
            .get(map)
            .ok_or_else(|| self.inconsistent(item, UNDEFINED))?;
        let map_record = if map_instance.is_complex() {
            None
        } else {
            exchange
                .records(map_instance)
                .next()
                .filter(|record| record.is("REPRESENTATION_MAP"))
        };
        let Some(map_record) = map_record else {
            return Err(unsupported(exchange, self.usage, map));
        };
        let map_params = record::exact_params(map_record, 2)
            .ok_or_else(|| self.inconsistent(map, MAP_LAYOUT))?;
        let (Some(origin), Some(mapped)) = (map_params[0].reference(), map_params[1].reference())
        else {
            return Err(self.inconsistent(map, MAP_LAYOUT));
        };
        if !self.component_shapes.contains(&mapped) {
            return Err(self.inconsistent(map, NOT_THE_COMPONENT));
        }
        self.sides = Some(Sides {
            component: mapped,
            assembly: holder,
        });
        let origin = self.frames.read(self.usage, origin, mapped)?;
        let target = self.frames.read(self.usage, target, holder)?;
        self.placed(&target, &origin)
    }

    /// `M(assembly) · M(component)⁻¹`, which must be finite.
    fn placed(
        &self,
        assembly: &Axis2Placement3d,
        component: &Axis2Placement3d,
    ) -> Result<Placed, PlacementError> {
        placed(self.exchange(), self.usage, assembly, component)
    }
}

/// `M(assembly) · M(component)⁻¹` for usage `#usage`, which must be finite.
fn placed(
    exchange: &Exchange<'_>,
    usage: u64,
    assembly: &Axis2Placement3d,
    component: &Axis2Placement3d,
) -> Result<Placed, PlacementError> {
    let inverse = component.to_rigid().inverse();
    if !is_finite(&inverse) {
        return Err(not_finite(exchange, usage, component.instance));
    }
    let transform = assembly.to_rigid() * inverse;
    if !is_finite(&transform) {
        return Err(not_finite(exchange, usage, assembly.instance));
    }
    Ok(Placed {
        transform,
        overflow: not_finite(exchange, usage, assembly.instance),
    })
}

/// A transform that leaves the range of an `f64`, reported at the frame
/// whose location takes it there.
pub(super) fn not_finite(exchange: &Exchange<'_>, usage: u64, frame: u64) -> PlacementError {
    PlacementError::Frame {
        usage,
        error: GeometryError {
            instance: frame,
            entity: entity(exchange, frame),
            attribute: Some("location"),
            kind: GeometryErrorKind::NotFinite,
        },
    }
}

/// The transform a mapped item `#mapped_item` states, as for a usage placed
/// through one: `M(target) · M(origin)⁻¹`, the frame `#origin` read in the
/// context of `#origin_representation` (the map's), `#target` in that of
/// `#target_representation` (the one listing the mapped item). Errors name
/// `#mapped_item` where they name a usage.
pub(super) fn mapped_item_transform(
    graph: &Graph<'_>,
    mapped_item: u64,
    (origin, origin_representation): (u64, u64),
    (target, target_representation): (u64, u64),
) -> Result<Rigid, PlacementError> {
    let mut frames = Frames {
        graph,
        readers: HashMap::new(),
        items: HashMap::new(),
    };
    let origin = frames.read(mapped_item, origin, origin_representation)?;
    let target = frames.read(mapped_item, target, target_representation)?;
    placed(graph.exchange(), mapped_item, &target, &origin).map(|placed| placed.transform)
}

fn entity(exchange: &Exchange<'_>, id: u64) -> String {
    exchange
        .get(id)
        .map(|instance| record::entity(exchange, instance))
        .unwrap_or_default()
}

fn unsupported(exchange: &Exchange<'_>, usage: u64, instance: u64) -> PlacementError {
    PlacementError::Unsupported {
        usage,
        instance,
        entity: entity(exchange, instance),
    }
}

/// Why a usage, or an occurrence, has no transform.
///
/// Each variant names the usage concerned by the `#id` of its
/// `next_assembly_usage_occurrence` (or other assembly usage), and, where
/// one instance is at fault, that instance. An occurrence carries the first
/// refusal on its path from the root. From
/// [`ShapeItem::transform`](super::ShapeItem::transform), `usage` is the
/// `#id` of the mapped item that could not be read.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[non_exhaustive]
pub enum PlacementError {
    /// Nothing places the usage: no `context_dependent_shape_representation`
    /// ties it to a relationship, and its assembly's shape does not hold
    /// exactly as many mapped items of the component as the assembly has
    /// usages of it.
    Unplaced {
        /// The usage.
        usage: u64,
    },
    /// A quantified usage whose quantity is not exactly 1, or cannot be read
    /// (`None`): it stands for several occurrences, which one placement
    /// cannot place.
    Quantified {
        /// The usage.
        usage: u64,
        /// Its quantity, if it can be read.
        quantity: Option<f64>,
    },
    /// The relationship states no transformation: a plain
    /// `shape_representation_relationship`.
    NoTransformation {
        /// The usage.
        usage: u64,
        /// The relationship.
        relationship: u64,
    },
    /// Valid STEP that is not read as a placement: a
    /// `cartesian_transformation_operator` of any dimension, a
    /// `functionally_defined_transformation`, any complex transformation or
    /// map, a simple relationship that is not a `representation_relationship`
    /// (complex ones are read), a complex mapped item with more than a
    /// representation item's partials, and a transform item, mapping origin
    /// or mapping target that is not an `axis2_placement_3d`.
    Unsupported {
        /// The usage.
        usage: u64,
        /// The instance not read.
        instance: u64,
        /// Its upper-cased entity types, joined with `+`.
        entity: String,
    },
    /// The file contradicts itself or says too little: `rep_1` and `rep_2`
    /// are not exactly one of the component's and one of the assembly's
    /// shape representations; `transform_item_k` is not listed directly in
    /// `rep_k`'s items; the usage has more than one
    /// `context_dependent_shape_representation`; not exactly one of the
    /// assembly's shape representations lists a mapped item, or one mapped
    /// item places more than one usage; a path places a definition through
    /// one representation and holds its components in another; a root's
    /// usages disagree on its representation; or a record does not have the
    /// attributes the schema gives it.
    Inconsistent {
        /// The usage.
        usage: u64,
        /// The instance at fault.
        instance: u64,
        /// What is wrong.
        reason: &'static str,
    },
    /// Stated, but ISO 10303 and Open CASCADE 8.0.1 place it differently, so
    /// a reading cannot be certain; see [`DivergentReason`].
    Divergent {
        /// The usage.
        usage: u64,
        /// The transformation or frame concerned.
        instance: u64,
        /// Why the readings differ.
        reason: DivergentReason,
    },
    /// An `axis2_placement_3d` the placement needs cannot be read: a
    /// malformed frame, a non-zero location whose length unit cannot be read
    /// ([`GeometryErrorKind::Unit`]), or a transform that is not finite once
    /// composed ([`GeometryErrorKind::NotFinite`], at the frame whose
    /// location takes it out of range).
    Frame {
        /// The usage.
        usage: u64,
        /// Why the frame cannot be read.
        error: GeometryError,
    },
    /// The usage places a definition that is already on the path from the
    /// root: the assembly would contain itself.
    Cycle {
        /// The usage.
        usage: u64,
    },
    /// The usage lies more than 1024 levels below the root; its components
    /// are not listed.
    TooDeep {
        /// The usage.
        usage: u64,
    },
}

impl fmt::Display for PlacementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unplaced { usage } => write!(
                f,
                "usage #{usage}: nothing places it (no context_dependent_shape_representation, \
                 and not as many mapped items as usages)"
            ),
            Self::Quantified {
                usage,
                quantity: Some(quantity),
            } => write!(
                f,
                "usage #{usage}: a quantity of {quantity}, which one placement cannot place"
            ),
            Self::Quantified {
                usage,
                quantity: None,
            } => write!(
                f,
                "usage #{usage}: its quantity cannot be read, so it may stand for several \
                 occurrences"
            ),
            Self::NoTransformation {
                usage,
                relationship,
            } => write!(
                f,
                "usage #{usage}: relationship #{relationship} states no transformation"
            ),
            Self::Unsupported {
                usage,
                instance,
                entity,
            } => write!(
                f,
                "usage #{usage}: #{instance} ({entity}) is not read as a placement"
            ),
            Self::Inconsistent {
                usage,
                instance,
                reason,
            } => write!(f, "usage #{usage}: #{instance}: {reason}"),
            Self::Divergent {
                usage,
                instance,
                reason,
            } => write!(f, "usage #{usage}: #{instance}: {reason}"),
            Self::Frame { usage, error } => write!(f, "usage #{usage}: {error}"),
            Self::Cycle { usage } => write!(
                f,
                "usage #{usage} places a definition already on its path from the root"
            ),
            Self::TooDeep { usage } => write!(
                f,
                "usage #{usage} lies more than {MAX_ASSEMBLY_DEPTH} levels below the root"
            ),
        }
    }
}

/// Why ISO 10303 and Open CASCADE 8.0.1 read a placement differently; see
/// [`PlacementError::Divergent`] and `docs/ARCHITECTURE.md`, "Known
/// differences from Open CASCADE".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum DivergentReason {
    /// Neither item of an `item_defined_transformation` is the identity:
    /// ISO 10303 gives `M(assembly side) · M(component side)⁻¹`, Open
    /// CASCADE 8.0.1 `M(item_1)⁻¹ · M(item_2)`. They agree only when one of
    /// the two is the identity.
    BothFramesPlaced,
    /// A placement frame omits `ref_direction` and its axis is not exactly
    /// +Z: the schema's default x axis and Open CASCADE's differ (measured
    /// for axes −Z, ±X, ±Y and (1, 1, 1)), which turns the component about
    /// its axis.
    DefaultedRefDirection,
}

impl fmt::Display for DivergentReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BothFramesPlaced => {
                "neither transform item is the identity, and ISO 10303 and Open CASCADE compose \
                 such a pair differently"
            }
            Self::DefaultedRefDirection => {
                "the frame omits ref_direction and its axis is not +Z, so ISO 10303 and Open \
                 CASCADE build different x axes"
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fmt::Write as _;

    use super::*;
    use crate::p21::parse;

    /// Units and contexts: millimetres (#20, #23), inches (#21), and one
    /// that assigns only a plane angle unit (#22).
    const CONTEXTS: &str = "#1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
#2=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));
#3=DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.);
#4=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#1);
#5=(CONVERSION_BASED_UNIT('INCH',#4)LENGTH_UNIT()NAMED_UNIT(#3));
#20=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#2))REPRESENTATION_CONTEXT('',''));
#21=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#5,#2))REPRESENTATION_CONTEXT('',''));
#22=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#2))REPRESENTATION_CONTEXT('',''));
#23=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#2))REPRESENTATION_CONTEXT('',''));
";
    const MM: u64 = 20;
    const INCH: u64 = 21;
    const NO_LENGTH: u64 = 22;
    const MM_2: u64 = 23;

    const X: [f64; 3] = [1.0, 0.0, 0.0];
    const Y: [f64; 3] = [0.0, 1.0, 0.0];
    const Z: [f64; 3] = [0.0, 0.0, 1.0];

    /// A quarter turn about z: x goes to y.
    const QUARTER: [[f64; 3]; 3] = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];

    /// Writes small assemblies: definitions with shape representations, and
    /// usages placed by item-defined transformations or mapped items.
    /// Representations are written last, with the items listed in them.
    struct File {
        data: String,
        next: u64,
        representations: Vec<(u64, String, u64)>,
        items: HashMap<u64, Vec<String>>,
    }

    impl File {
        fn new() -> Self {
            Self {
                data: String::new(),
                next: 100,
                representations: Vec::new(),
                items: HashMap::new(),
            }
        }

        /// Writes `entity` as the next instance.
        fn add(&mut self, entity: &str) -> u64 {
            let id = self.next;
            self.next += 1;
            writeln!(self.data, "#{id}={entity};").unwrap();
            id
        }

        fn triple(v: [f64; 3]) -> String {
            format!("({:?},{:?},{:?})", v[0], v[1], v[2])
        }

        /// An `axis2_placement_3d`; `None` writes `$`.
        fn frame(
            &mut self,
            location: [f64; 3],
            axis: Option<[f64; 3]>,
            ref_direction: Option<[f64; 3]>,
        ) -> u64 {
            let point = self.add(&format!("CARTESIAN_POINT('',{})", Self::triple(location)));
            let mut direction = |v: Option<[f64; 3]>| {
                v.map_or("$".to_owned(), |v| {
                    format!(
                        "#{}",
                        self.add(&format!("DIRECTION('',{})", Self::triple(v)))
                    )
                })
            };
            let axis = direction(axis);
            let ref_direction = direction(ref_direction);
            self.add(&format!(
                "AXIS2_PLACEMENT_3D('',#{point},{axis},{ref_direction})"
            ))
        }

        fn identity(&mut self) -> u64 {
            self.frame([0.0; 3], Some(Z), Some(X))
        }

        /// A product definition with a shape representation of `entity` in
        /// `context`: (definition, representation).
        fn definition_with(&mut self, name: &str, entity: &str, context: u64) -> (u64, u64) {
            let definition = self.add(&format!("PRODUCT_DEFINITION('{name}','',$,$)"));
            let shape = self.add(&format!("PRODUCT_DEFINITION_SHAPE('','',#{definition})"));
            let representation = self.next;
            self.next += 1;
            self.add(&format!(
                "SHAPE_DEFINITION_REPRESENTATION(#{shape},#{representation})"
            ));
            self.representations
                .push((representation, entity.to_owned(), context));
            (definition, representation)
        }

        fn definition(&mut self, name: &str, context: u64) -> (u64, u64) {
            self.definition_with(name, "SHAPE_REPRESENTATION", context)
        }

        /// Another shape representation of `definition`.
        fn representation(&mut self, definition: u64, context: u64) -> u64 {
            let shape = self.add(&format!("PRODUCT_DEFINITION_SHAPE('','',#{definition})"));
            let representation = self.next;
            self.next += 1;
            self.add(&format!(
                "SHAPE_DEFINITION_REPRESENTATION(#{shape},#{representation})"
            ));
            self.representations
                .push((representation, "SHAPE_REPRESENTATION".to_owned(), context));
            representation
        }

        /// Lists `item` among `representation`'s items.
        fn list(&mut self, representation: u64, item: &str) {
            self.items
                .entry(representation)
                .or_default()
                .push(item.to_owned());
        }

        fn list_id(&mut self, representation: u64, item: u64) {
            self.list(representation, &format!("#{item}"));
        }

        fn usage(&mut self, assembly: u64, component: u64) -> u64 {
            let name = self.next;
            self.add(&format!(
                "NEXT_ASSEMBLY_USAGE_OCCURRENCE('{name}','','',#{assembly},#{component},$)"
            ))
        }

        /// Ties `usage` to the relationship `relationship`, written with
        /// `{id}` standing for its own `#id`.
        fn link(&mut self, usage: u64, relationship: &str) -> u64 {
            let id = self.next;
            self.add(&relationship.replace("{id}", &id.to_string()));
            let shape = self.add(&format!("PRODUCT_DEFINITION_SHAPE('','',#{usage})"));
            self.add(&format!(
                "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#{id},#{shape})"
            ));
            id
        }

        /// A complex representation relationship with transformation
        /// `#transformation`, from `rep_1` to `rep_2`.
        fn relate(&mut self, usage: u64, rep_1: u64, rep_2: u64, transformation: u64) -> u64 {
            self.link(
                usage,
                &format!(
                    "(REPRESENTATION_RELATIONSHIP('','',#{rep_1},#{rep_2})\
                     REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#{transformation})\
                     SHAPE_REPRESENTATION_RELATIONSHIP())"
                ),
            )
        }

        /// Places `usage` by an item-defined transformation from `item_1` to
        /// `item_2`, listing each in its representation: `rep_1` is the
        /// component's unless `reversed`.
        fn place(
            &mut self,
            usage: u64,
            (rep_1, item_1): (u64, u64),
            (rep_2, item_2): (u64, u64),
        ) -> u64 {
            self.list_id(rep_1, item_1);
            self.list_id(rep_2, item_2);
            let transformation = self.add(&format!(
                "ITEM_DEFINED_TRANSFORMATION('','',#{item_1},#{item_2})"
            ));
            self.relate(usage, rep_1, rep_2, transformation);
            transformation
        }

        /// A component of `assembly` placed at `frame` in `assembly_rep`, by
        /// an identity frame in its own representation.
        fn component(
            &mut self,
            (assembly, assembly_rep): (u64, u64),
            (component, component_rep): (u64, u64),
            frame: u64,
        ) -> u64 {
            let usage = self.usage(assembly, component);
            let origin = self.identity();
            self.place(usage, (component_rep, origin), (assembly_rep, frame));
            usage
        }

        /// A mapped item of `component_rep` in `assembly_rep`, from `origin`
        /// to `target`.
        fn mapped(
            &mut self,
            assembly_rep: u64,
            component_rep: u64,
            origin: u64,
            target: u64,
        ) -> u64 {
            let map = self.add(&format!("REPRESENTATION_MAP(#{origin},#{component_rep})"));
            let item = self.add(&format!("MAPPED_ITEM('',#{map},#{target})"));
            self.list_id(assembly_rep, item);
            item
        }

        fn text(&self) -> String {
            let mut text = format!(
                "ISO-10303-21;HEADER;FILE_SCHEMA(('AP242'));ENDSEC;DATA;\n{CONTEXTS}{}",
                self.data
            );
            for (id, entity, context) in &self.representations {
                let items = self.items.get(id).map_or(String::new(), |i| i.join(","));
                writeln!(text, "#{id}={entity}('',({items}),#{context});").unwrap();
            }
            text + "ENDSEC;END-ISO-10303-21;"
        }
    }

    /// The structure and placements of `src`, kept for the test's lifetime.
    fn read(src: &str) -> (&'static ProductStructure, Placements<'static>) {
        let src: &'static [u8] = Box::leak(src.as_bytes().to_vec().into_boxed_slice());
        let graph: &'static Graph<'static> = Box::leak(Box::new(Graph::build(parse(src).unwrap())));
        let structure: &'static ProductStructure =
            Box::leak(Box::new(ProductStructure::new(graph)));
        (structure, Placements::new(graph, structure))
    }

    fn read_file(file: &File) -> (&'static ProductStructure, Placements<'static>) {
        read(&file.text())
    }

    /// The index of the usage `#id`.
    fn index(structure: &ProductStructure, id: u64) -> usize {
        structure
            .usages()
            .iter()
            .position(|u| u.instance == id)
            .unwrap()
    }

    /// The index of the definition `#id`.
    fn definition(structure: &ProductStructure, id: u64) -> usize {
        structure
            .definitions()
            .iter()
            .position(|d| d.instance == id)
            .unwrap()
    }

    fn usage_of(file: &File, id: u64) -> Result<Rigid, PlacementError> {
        let (structure, placements) = read_file(file);
        placements.usage(index(structure, id))
    }

    fn close(a: &Rigid, b: &Rigid) -> bool {
        let rotation = a
            .rotation
            .iter()
            .flatten()
            .zip(b.rotation.iter().flatten())
            .all(|(x, y)| (x - y).abs() <= 1e-15);
        let translation = a
            .translation
            .iter()
            .zip(b.translation)
            .all(|(x, y)| (x - y).abs() <= 1e-12 * y.abs().max(1.0));
        rotation && translation
    }

    #[track_caller]
    fn assert_close(found: &Result<Rigid, PlacementError>, expected: Rigid) {
        let found = found.as_ref().unwrap();
        assert!(close(found, &expected), "{found:?} is not {expected:?}");
    }

    fn rigid(rotation: [[f64; 3]; 3], translation: [f64; 3]) -> Rigid {
        Rigid {
            rotation,
            translation,
        }
    }

    fn moved(translation: [f64; 3]) -> Rigid {
        rigid(Rigid::IDENTITY.rotation, translation)
    }

    #[test]
    fn an_item_defined_transformation_carries_the_component_onto_the_assembly() {
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", MM_2);
        // The assembly frame turns x onto y and moves to (10, 20, 30).
        let frame = file.frame([10.0, 20.0, 30.0], Some(Z), Some(Y));
        let usage = file.component(assembly, part, frame);
        let (structure, placements) = read_file(&file);
        let expected = rigid(QUARTER, [10.0, 20.0, 30.0]);
        assert_eq!(placements.usage(index(structure, usage)), Ok(expected));
        assert_eq!(expected.apply_point([1.0, 0.0, 0.0]), [10.0, 21.0, 30.0]);

        let occurrences: Vec<Occurrence> = placements.occurrences(0).collect();
        assert_eq!(
            occurrences,
            [
                Occurrence {
                    path: vec![],
                    definition: 0,
                    representation: Some(assembly.1),
                    transform: Ok(Rigid::IDENTITY),
                },
                Occurrence {
                    path: vec![0],
                    definition: 1,
                    representation: Some(part.1),
                    transform: Ok(expected),
                },
            ]
        );
    }

    /// Pro/ENGINEER writes the assembly's representation as `rep_1` and
    /// its non-identity frame as `transform_item_1`: the usage decides which
    /// side is the component's, so the transform is `M(item_1)`, not its
    /// inverse.
    #[test]
    fn a_reversed_relationship_is_read_by_the_usage() {
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", MM_2);
        let usage = file.usage(assembly.0, part.0);
        let frame = file.frame([5.0, 0.0, 0.0], Some(Z), Some(Y));
        let origin = file.identity();
        file.place(usage, (assembly.1, frame), (part.1, origin));
        let (structure, placements) = read_file(&file);
        assert_eq!(
            structure.usages()[0]
                .placement
                .as_ref()
                .and_then(Placement::reversed),
            Some(true)
        );
        let expected = rigid(QUARTER, [5.0, 0.0, 0.0]);
        assert_eq!(placements.usage(0), Ok(expected));
        assert_ne!(Ok(expected.inverse()), placements.usage(0));
        let part_occurrence = placements.occurrences(0).nth(1).unwrap();
        assert_eq!(part_occurrence.representation, Some(part.1));
    }

    #[test]
    fn transform_items_must_be_items_of_their_own_representations() {
        // transform_item_1 listed only in rep_2, as no fixture does.
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", MM_2);
        let usage = file.usage(assembly.0, part.0);
        let origin = file.identity();
        let frame = file.frame([1.0, 0.0, 0.0], None, None);
        file.list_id(assembly.1, origin);
        file.list_id(assembly.1, frame);
        let transformation = file.add(&format!(
            "ITEM_DEFINED_TRANSFORMATION('','',#{origin},#{frame})"
        ));
        file.relate(usage, part.1, assembly.1, transformation);
        assert_eq!(
            usage_of(&file, usage),
            Err(PlacementError::Inconsistent {
                usage,
                instance: origin,
                reason: ITEM_1,
            })
        );

        // Listed in both, as Open CASCADE writes a shared identity frame:
        // read.
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", MM_2);
        let usage = file.usage(assembly.0, part.0);
        let origin = file.identity();
        let frame = file.frame([1.0, 0.0, 0.0], None, None);
        file.list_id(assembly.1, origin);
        file.place(usage, (part.1, origin), (assembly.1, frame));
        assert_eq!(usage_of(&file, usage), Ok(moved([1.0, 0.0, 0.0])));

        // transform_item_2 not listed at all.
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", MM_2);
        let usage = file.usage(assembly.0, part.0);
        let origin = file.identity();
        let frame = file.frame([1.0, 0.0, 0.0], None, None);
        file.list_id(part.1, origin);
        let transformation = file.add(&format!(
            "ITEM_DEFINED_TRANSFORMATION('','',#{origin},#{frame})"
        ));
        file.relate(usage, part.1, assembly.1, transformation);
        assert_eq!(
            usage_of(&file, usage),
            Err(PlacementError::Inconsistent {
                usage,
                instance: frame,
                reason: ITEM_2,
            })
        );
    }

    #[test]
    fn a_mapped_item_maps_its_origin_onto_its_target() {
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", MM_2);
        let usage = file.usage(assembly.0, part.0);
        let origin = file.identity();
        let target = file.frame([0.0, 0.0, 7.0], Some(Z), Some(Y));
        file.mapped(assembly.1, part.1, origin, target);
        let (structure, placements) = read_file(&file);
        assert!(matches!(
            structure.usages()[0].placement,
            Some(Placement::MappedItem { .. })
        ));
        assert_eq!(
            placements.usage(index(structure, usage)),
            Ok(rigid(QUARTER, [0.0, 0.0, 7.0]))
        );
        let part_occurrence = placements.occurrences(0).nth(1).unwrap();
        assert_eq!(part_occurrence.representation, Some(part.1));

        // An origin that is not the identity is taken off first: ISO 10303
        // and Open CASCADE agree on mapped items.
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", MM_2);
        let usage = file.usage(assembly.0, part.0);
        let origin = file.frame([1.0, 0.0, 0.0], None, None);
        let target = file.frame([0.0, 0.0, 7.0], Some(Z), Some(Y));
        file.mapped(assembly.1, part.1, origin, target);
        assert_close(&usage_of(&file, usage), rigid(QUARTER, [0.0, -1.0, 7.0]));
    }

    /// A mapped item places one occurrence. Two usages that both claim it
    /// are refused, whether the assembly's shape is defined twice (so its
    /// one mapped item is counted twice) or two components share the
    /// representation it maps.
    #[test]
    fn a_mapped_item_places_only_one_usage() {
        let shared = |usages: &[u64], item: u64, file: &File| {
            for &usage in usages {
                assert_eq!(
                    usage_of(file, usage),
                    Err(PlacementError::Inconsistent {
                        usage,
                        instance: item,
                        reason: SHARED_MAPPED_ITEM,
                    })
                );
            }
        };

        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let shape = file.add(&format!("PRODUCT_DEFINITION_SHAPE('','',#{})", assembly.0));
        file.add(&format!(
            "SHAPE_DEFINITION_REPRESENTATION(#{shape},#{})",
            assembly.1
        ));
        let part = file.definition("p", MM);
        let first = file.usage(assembly.0, part.0);
        let second = file.usage(assembly.0, part.0);
        let origin = file.identity();
        let target = file.frame([10.0, 0.0, 0.0], None, None);
        let item = file.mapped(assembly.1, part.1, origin, target);
        shared(&[first, second], item, &file);

        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", MM);
        let twin = file.add("PRODUCT_DEFINITION('q','',$,$)");
        let shape = file.add(&format!("PRODUCT_DEFINITION_SHAPE('','',#{twin})"));
        file.add(&format!(
            "SHAPE_DEFINITION_REPRESENTATION(#{shape},#{})",
            part.1
        ));
        let first = file.usage(assembly.0, part.0);
        let second = file.usage(assembly.0, twin);
        let origin = file.identity();
        let target = file.frame([10.0, 0.0, 0.0], None, None);
        let item = file.mapped(assembly.1, part.1, origin, target);
        shared(&[first, second], item, &file);
    }

    /// Each frame is read in the context its position assigns: an inch
    /// component's frame in inches, the assembly's in millimetres.
    #[test]
    fn frames_are_read_in_the_units_of_their_own_representations() {
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", INCH);
        let usage = file.usage(assembly.0, part.0);
        let origin = file.frame([1.0, 0.0, 0.0], Some(Z), Some(X));
        let frame = file.identity();
        file.place(usage, (part.1, origin), (assembly.1, frame));
        assert_eq!(usage_of(&file, usage), Ok(moved([-25.4, 0.0, 0.0])));

        // A mapped item: the origin in the mapped representation's units,
        // the target in those of the representation that lists the item.
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", INCH);
        let usage = file.usage(assembly.0, part.0);
        let origin = file.frame([1.0, 0.0, 0.0], None, None);
        let target = file.frame([10.0, 0.0, 0.0], None, None);
        file.mapped(assembly.1, part.1, origin, target);
        assert_close(&usage_of(&file, usage), moved([10.0 - 25.4, 0.0, 0.0]));

        // A zero location needs no unit; any other does.
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", NO_LENGTH);
        let placed = file.usage(assembly.0, part.0);
        let origin = file.identity();
        let frame = file.frame([3.0, 0.0, 0.0], None, None);
        file.place(placed, (part.1, origin), (assembly.1, frame));
        let refused = file.usage(assembly.0, part.0);
        let origin = file.frame([3.0, 0.0, 0.0], None, None);
        let frame = file.identity();
        file.place(refused, (part.1, origin), (assembly.1, frame));
        let (structure, placements) = read_file(&file);
        assert_eq!(
            placements.usage(index(structure, placed)),
            Ok(moved([3.0, 0.0, 0.0]))
        );
        let Err(PlacementError::Frame { usage, error }) =
            placements.usage(index(structure, refused))
        else {
            panic!("a unit is needed");
        };
        assert_eq!(
            (usage, error.instance),
            (refused, origin - 1),
            "at the point"
        );
        assert!(matches!(
            error.kind,
            GeometryErrorKind::Unit(UnitError::Missing { .. })
        ));
    }

    #[test]
    fn usages_without_a_single_placement_are_refused() {
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", MM_2);
        let unplaced = file.usage(assembly.0, part.0);
        // A plain shape_representation_relationship: no transformation.
        let plain = file.usage(assembly.0, part.0);
        let relationship = file.link(
            plain,
            &format!(
                "SHAPE_REPRESENTATION_RELATIONSHIP('','',#{},#{})",
                part.1, assembly.1
            ),
        );
        // Two context_dependent_shape_representations for one usage.
        let twice = file.usage(assembly.0, part.0);
        let frame = file.frame([1.0, 0.0, 0.0], None, None);
        let origin = file.identity();
        file.place(twice, (part.1, origin), (assembly.1, frame));
        let second = file.place(twice, (part.1, origin), (assembly.1, frame));
        // A relationship between representations of other definitions.
        let other = file.definition("o", MM);
        let elsewhere = file.usage(assembly.0, part.0);
        let origin = file.identity();
        file.place(elsewhere, (other.1, origin), (assembly.1, frame));
        let (structure, placements) = read_file(&file);
        let read = |id| placements.usage(index(structure, id));
        assert_eq!(
            read(unplaced),
            Err(PlacementError::Unplaced { usage: unplaced })
        );
        assert_eq!(
            read(plain),
            Err(PlacementError::NoTransformation {
                usage: plain,
                relationship
            })
        );
        let Err(PlacementError::Inconsistent {
            usage,
            instance,
            reason,
        }) = read(twice)
        else {
            panic!("two links");
        };
        assert_eq!((usage, reason), (twice, MORE_THAN_ONE_LINK));
        // The second link, written after its relationship.
        assert!(instance > second, "{instance}");
        assert!(matches!(
            read(elsewhere),
            Err(PlacementError::Inconsistent { reason: SIDES, .. })
        ));
    }

    #[test]
    fn quantified_usages_place_only_one() {
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", MM_2);
        let quantified = |file: &mut File, quantity: &str| {
            let measure = file.add(&format!("MEASURE_WITH_UNIT({quantity},#1)"));
            let usage = file.add(&format!(
                "(ASSEMBLY_COMPONENT_USAGE($)NEXT_ASSEMBLY_USAGE_OCCURRENCE()\
                 PRODUCT_DEFINITION_RELATIONSHIP('q','','',#{},#{})PRODUCT_DEFINITION_USAGE()\
                 QUANTIFIED_ASSEMBLY_COMPONENT_USAGE(#{measure}))",
                assembly.0, part.0
            ));
            let origin = file.identity();
            let frame = file.frame([1.0, 0.0, 0.0], None, None);
            file.place(usage, (part.1, origin), (assembly.1, frame));
            usage
        };
        let four = quantified(&mut file, "COUNT_MEASURE(4.)");
        let one = quantified(&mut file, "COUNT_MEASURE(1.)");
        let unreadable = quantified(&mut file, "'four'");
        let (structure, placements) = read_file(&file);
        let read = |id| placements.usage(index(structure, id));
        assert_eq!(
            read(four),
            Err(PlacementError::Quantified {
                usage: four,
                quantity: Some(4.0)
            })
        );
        assert_eq!(read(one), Ok(moved([1.0, 0.0, 0.0])));
        assert_eq!(
            read(unreadable),
            Err(PlacementError::Quantified {
                usage: unreadable,
                quantity: None
            })
        );
        // The refused usage still names its representation.
        let occurrence = placements.occurrences(0).nth(1).unwrap();
        assert_eq!(occurrence.path, [index(structure, four)]);
        assert_eq!(occurrence.representation, Some(part.1));
    }

    #[test]
    fn transformations_other_than_two_frames_are_not_read() {
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", MM_2);
        // A cartesian transformation operator as the relationship's
        // transformation.
        let operator_usage = file.usage(assembly.0, part.0);
        let origin = file.identity();
        let operator = file.add(&format!(
            "CARTESIAN_TRANSFORMATION_OPERATOR_3D('','','',$,$,#{origin},$,$)"
        ));
        file.relate(operator_usage, part.1, assembly.1, operator);
        // A functionally defined transformation.
        let functional_usage = file.usage(assembly.0, part.0);
        let functional = file.add("FUNCTIONALLY_DEFINED_TRANSFORMATION('','')");
        file.relate(functional_usage, part.1, assembly.1, functional);
        // A 2D frame as a transform item.
        let flat_usage = file.usage(assembly.0, part.0);
        let point = file.add("CARTESIAN_POINT('',(0.,0.))");
        let flat = file.add(&format!("AXIS2_PLACEMENT_2D('',#{point},$)"));
        let frame = file.identity();
        file.place(flat_usage, (part.1, flat), (assembly.1, frame));
        // A mapped item whose target is an operator.
        let other = file.definition("o", MM);
        let mapped_usage = file.usage(assembly.0, other.0);
        let origin = file.identity();
        let target = file.add(&format!(
            "CARTESIAN_TRANSFORMATION_OPERATOR_3D('','','',$,$,#{origin},2.,$)"
        ));
        file.mapped(assembly.1, other.1, origin, target);
        let (structure, placements) = read_file(&file);
        let read = |id| placements.usage(index(structure, id));
        let unsupported = |usage, instance, entity: &str| {
            Err(PlacementError::Unsupported {
                usage,
                instance,
                entity: entity.to_owned(),
            })
        };
        assert_eq!(
            read(operator_usage),
            unsupported(
                operator_usage,
                operator,
                "CARTESIAN_TRANSFORMATION_OPERATOR_3D"
            )
        );
        assert_eq!(
            read(functional_usage),
            unsupported(
                functional_usage,
                functional,
                "FUNCTIONALLY_DEFINED_TRANSFORMATION"
            )
        );
        assert_eq!(
            read(flat_usage),
            unsupported(flat_usage, flat, "AXIS2_PLACEMENT_2D")
        );
        assert_eq!(
            read(mapped_usage),
            unsupported(mapped_usage, target, "CARTESIAN_TRANSFORMATION_OPERATOR_3D")
        );
    }

    /// Where ISO 10303 and Open CASCADE 8.0.1 read a placement differently,
    /// it is refused rather than read either way.
    #[test]
    fn divergent_placements_are_refused() {
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", MM_2);
        // Neither transform item is the identity.
        let both = file.usage(assembly.0, part.0);
        let origin = file.frame([10.0, 0.0, 0.0], Some(X), Some(Y));
        let frame = file.frame([0.0, 0.0, -10.0], None, None);
        let both_transformation = file.place(both, (part.1, origin), (assembly.1, frame));
        // An axis of (0, 0, -1) without a ref_direction.
        let defaulted = file.usage(assembly.0, part.0);
        let origin = file.identity();
        let minus_z = file.frame([1.0, 0.0, 0.0], Some([0.0, 0.0, -1.0]), None);
        file.place(defaulted, (part.1, origin), (assembly.1, minus_z));
        // An axis of +Z, stated at any length, or no axis at all: both
        // readings agree on x.
        let plus_z = file.usage(assembly.0, part.0);
        let origin = file.identity();
        let frame = file.frame([1.0, 0.0, 0.0], Some([0.0, 0.0, 2.5]), None);
        file.place(plus_z, (part.1, origin), (assembly.1, frame));
        let neither = file.usage(assembly.0, part.0);
        let origin = file.frame([0.0; 3], None, None);
        let frame = file.frame([1.0, 0.0, 0.0], None, None);
        file.place(neither, (part.1, origin), (assembly.1, frame));
        let (structure, placements) = read_file(&file);
        let read = |id| placements.usage(index(structure, id));
        assert_eq!(
            read(both),
            Err(PlacementError::Divergent {
                usage: both,
                instance: both_transformation,
                reason: DivergentReason::BothFramesPlaced,
            })
        );
        assert_eq!(
            read(defaulted),
            Err(PlacementError::Divergent {
                usage: defaulted,
                instance: minus_z,
                reason: DivergentReason::DefaultedRefDirection,
            })
        );
        assert_eq!(read(plus_z), Ok(moved([1.0, 0.0, 0.0])));
        assert_eq!(read(neither), Ok(moved([1.0, 0.0, 0.0])));
    }

    #[test]
    fn frames_that_cannot_be_read_refuse_their_usage() {
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", MM_2);
        let usage = file.usage(assembly.0, part.0);
        let origin = file.identity();
        let frame = file.frame([1.0, 0.0, 0.0], Some([0.0, 0.0, 0.0]), None);
        file.place(usage, (part.1, origin), (assembly.1, frame));
        let Err(PlacementError::Frame { error, .. }) = usage_of(&file, usage) else {
            panic!("a zero axis");
        };
        assert_eq!(error.kind, GeometryErrorKind::ZeroDirection);
        assert_eq!(error.instance, frame - 1, "reported at the direction");

        // A location so large that the transform leaves the range of f64.
        let mut file = File::new();
        let assembly = file.definition("a", MM);
        let part = file.definition("p", MM_2);
        let usage = file.usage(assembly.0, part.0);
        let origin = file.frame([1.3e308, 1.3e308, 0.0], Some([1.0, 1.0, 0.0]), Some(Z));
        let frame = file.identity();
        file.place(usage, (part.1, origin), (assembly.1, frame));
        let Err(PlacementError::Frame { error, .. }) = usage_of(&file, usage) else {
            panic!("not finite");
        };
        assert_eq!(
            (error.instance, error.kind),
            (origin, GeometryErrorKind::NotFinite)
        );
    }

    /// A sub-assembly used twice occurs twice, each time with its own
    /// transform, composed from the root down.
    #[test]
    fn occurrences_compose_from_the_root_down() {
        let mut file = File::new();
        let top = file.definition("top", MM);
        let pair = file.definition("pair", MM_2);
        let bolt = file.definition("bolt", INCH);
        let left = file.frame([-100.0, 0.0, 0.0], None, None);
        let right = file.frame([100.0, 0.0, 0.0], Some(Z), Some(Y));
        let first = file.component(top, pair, left);
        let second = file.component(top, pair, right);
        let up = file.frame([0.0, 0.0, 1.0], None, None);
        let bolt_usage = file.component(pair, bolt, up);
        let (structure, placements) = read_file(&file);
        let root = definition(structure, top.0);
        assert_eq!(structure.roots(), [root]);
        let occurrences: Vec<Occurrence> = placements.occurrences(root).collect();
        let paths: Vec<Vec<u64>> = occurrences
            .iter()
            .map(|o| {
                o.path
                    .iter()
                    .map(|&u| structure.usages()[u].instance)
                    .collect()
            })
            .collect();
        assert_eq!(
            paths,
            [
                vec![],
                vec![first],
                vec![first, bolt_usage],
                vec![second],
                vec![second, bolt_usage]
            ]
        );
        let representations: Vec<Option<u64>> =
            occurrences.iter().map(|o| o.representation).collect();
        assert_eq!(
            representations,
            [
                Some(top.1),
                Some(pair.1),
                Some(bolt.1),
                Some(pair.1),
                Some(bolt.1)
            ]
        );
        assert_close(&occurrences[2].transform, moved([-100.0, 0.0, 1.0]));
        assert_close(&occurrences[4].transform, rigid(QUARTER, [100.0, 0.0, 1.0]));
        assert_eq!(
            occurrences[4].transform,
            Ok(placements.usage(index(structure, second)).unwrap()
                * placements.usage(index(structure, bolt_usage)).unwrap())
        );
    }

    #[test]
    fn separate_roots_are_unrelated_frames() {
        let mut file = File::new();
        let first = file.definition("first", MM);
        let second = file.definition("second", MM_2);
        let part = file.definition("p", INCH);
        let lone = file.definition("lone", MM);
        let frame = file.frame([1.0, 0.0, 0.0], None, None);
        file.component(first, part, frame);
        let frame = file.frame([2.0, 0.0, 0.0], None, None);
        file.component(second, part, frame);
        let (structure, placements) = read_file(&file);
        let roots: Vec<u64> = structure
            .roots()
            .iter()
            .map(|&r| structure.definitions()[r].instance)
            .collect();
        assert_eq!(roots, [first.0, second.0, lone.0]);
        let parts: Vec<Result<Rigid, PlacementError>> = structure.roots()[..2]
            .iter()
            .map(|&root| placements.occurrences(root).nth(1).unwrap().transform)
            .collect();
        assert_eq!(
            parts,
            [Ok(moved([1.0, 0.0, 0.0])), Ok(moved([2.0, 0.0, 0.0]))]
        );
        // A part with one shape representation and no usages.
        let only: Vec<Occurrence> = placements.occurrences(structure.roots()[2]).collect();
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].representation, Some(lone.1));
    }

    #[test]
    fn a_path_keeps_one_representation_per_definition() {
        // The sub-assembly is placed through one representation and holds
        // its component in another.
        let mut file = File::new();
        let top = file.definition("top", MM);
        let sub = file.definition("sub", MM_2);
        let other = file.representation(sub.0, MM_2);
        let part = file.definition("p", INCH);
        let frame = file.frame([1.0, 0.0, 0.0], None, None);
        file.component(top, sub, frame);
        let frame = file.frame([2.0, 0.0, 0.0], None, None);
        let inner = file.component((sub.0, other), part, frame);
        let (structure, placements) = read_file(&file);
        assert_eq!(
            placements.usage(index(structure, inner)),
            Ok(moved([2.0, 0.0, 0.0])),
            "the usage itself reads"
        );
        let occurrences: Vec<Occurrence> = placements.occurrences(0).collect();
        assert_eq!(
            occurrences[2].transform,
            Err(PlacementError::Inconsistent {
                usage: inner,
                instance: other,
                reason: CHANGES_REPRESENTATION,
            })
        );
        assert_eq!(occurrences[2].representation, Some(part.1));

        // A root whose usages place components in two of its
        // representations.
        let mut file = File::new();
        let top = file.definition("top", MM);
        let second = file.representation(top.0, MM);
        let part = file.definition("p", INCH);
        let frame = file.frame([1.0, 0.0, 0.0], None, None);
        let first_usage = file.component(top, part, frame);
        let frame = file.frame([2.0, 0.0, 0.0], None, None);
        let second_usage = file.component((top.0, second), part, frame);
        let (_, placements) = read_file(&file);
        let occurrences: Vec<Occurrence> = placements.occurrences(0).collect();
        assert_eq!(occurrences[0].representation, None);
        assert_eq!(occurrences[0].transform, Ok(Rigid::IDENTITY));
        assert_eq!(
            occurrences[1..]
                .iter()
                .map(|o| o.transform.clone())
                .collect::<Vec<_>>(),
            [
                Err(PlacementError::Inconsistent {
                    usage: first_usage,
                    instance: top.1,
                    reason: AMBIGUOUS_ROOT,
                }),
                Err(PlacementError::Inconsistent {
                    usage: second_usage,
                    instance: second,
                    reason: AMBIGUOUS_ROOT,
                }),
            ]
        );
    }

    #[test]
    fn an_occurrence_carries_the_first_refusal_on_its_path() {
        let mut file = File::new();
        let top = file.definition("top", MM);
        let sub = file.definition("sub", MM_2);
        let part = file.definition("p", INCH);
        let unplaced = file.usage(top.0, sub.0);
        let frame = file.frame([2.0, 0.0, 0.0], None, None);
        file.component(sub, part, frame);
        let (_, placements) = read_file(&file);
        let transforms: Vec<Result<Rigid, PlacementError>> =
            placements.occurrences(0).map(|o| o.transform).collect();
        let refused = Err(PlacementError::Unplaced { usage: unplaced });
        assert_eq!(transforms, [Ok(Rigid::IDENTITY), refused.clone(), refused]);
    }

    #[test]
    fn cycles_and_depth_end_a_path() {
        // Every definition is someone's component.
        let src = "ISO-10303-21;HEADER;FILE_SCHEMA(('X'));ENDSEC;DATA;
#1=PRODUCT_DEFINITION('a','',$,$);
#2=PRODUCT_DEFINITION('b','',$,$);
#3=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','','',#1,#2,$);
#4=NEXT_ASSEMBLY_USAGE_OCCURRENCE('2','','',#2,#1,$);
ENDSEC;END-ISO-10303-21;";
        let (structure, placements) = read(src);
        assert_eq!(structure.roots(), [0usize; 0]);
        let occurrences: Vec<Occurrence> = placements.occurrences(0).collect();
        assert_eq!(
            occurrences
                .iter()
                .map(|o| (o.path.clone(), o.transform.clone()))
                .collect::<Vec<_>>(),
            [
                (vec![], Ok(Rigid::IDENTITY)),
                (vec![0], Err(PlacementError::Unplaced { usage: 3 })),
                (vec![0, 1], Err(PlacementError::Cycle { usage: 4 })),
            ]
        );

        // A chain 1,100 deep, each level one millimetre along x.
        let mut file = File::new();
        let mut parent = file.definition("0", MM);
        let mut usages = Vec::new();
        for level in 1..=1_100 {
            let child = file.definition(&level.to_string(), MM);
            let frame = file.frame([1.0, 0.0, 0.0], None, None);
            usages.push(file.component(parent, child, frame));
            parent = child;
        }
        let (structure, placements) = read_file(&file);
        assert_eq!(structure.roots(), [0]);
        let occurrences: Vec<Occurrence> = placements.occurrences(0).collect();
        assert_eq!(occurrences.len(), 1 + MAX_ASSEMBLY_DEPTH + 1);
        let deepest = &occurrences[MAX_ASSEMBLY_DEPTH];
        assert_eq!(deepest.path.len(), MAX_ASSEMBLY_DEPTH);
        #[expect(clippy::cast_precision_loss, reason = "1024 is exact")]
        let depth = MAX_ASSEMBLY_DEPTH as f64;
        assert_eq!(deepest.transform, Ok(moved([depth, 0.0, 0.0])));
        let beyond = occurrences.last().unwrap();
        assert_eq!(beyond.path.len(), MAX_ASSEMBLY_DEPTH + 1);
        assert_eq!(
            beyond.transform,
            Err(PlacementError::TooDeep {
                usage: usages[MAX_ASSEMBLY_DEPTH]
            })
        );
    }

    /// Each level uses the next twice: 2^41 - 1 occurrences, of which the
    /// iterator computes only those asked for.
    #[test]
    fn occurrences_are_produced_lazily() {
        let mut file = File::new();
        let mut parent = file.definition("0", MM);
        for level in 1..=40 {
            let child = file.definition(&level.to_string(), MM);
            for _ in 0..2 {
                let frame = file.frame([1.0, 0.0, 0.0], None, None);
                file.component(parent, child, frame);
            }
            parent = child;
        }
        let (_, placements) = read_file(&file);
        let mut occurrences = placements.occurrences(0);
        let first: Vec<usize> = occurrences
            .by_ref()
            .take(42)
            .map(|o| o.path.len())
            .collect();
        assert_eq!(first, (0..=40).chain([40]).collect::<Vec<_>>());
        assert_eq!(occurrences.take(1_000).count(), 1_000);
    }

    /// A complex `MAPPED_ITEM` holds `(mapping_source, mapping_target)` in
    /// its partial record, and a complex representation lists it.
    #[test]
    fn a_complex_mapped_item_places_its_usage() {
        let mut file = File::new();
        let assembly = file.definition_with("a", "(REPRESENTATION_RELATIONSHIP_PLACEHOLDER)", MM);
        let part = file.definition("p", MM_2);
        let usage = file.usage(assembly.0, part.0);
        let origin = file.identity();
        let target = file.frame([0.0, 4.0, 0.0], None, None);
        let map = file.add(&format!("REPRESENTATION_MAP(#{origin},#{})", part.1));
        let item = file.add(&format!(
            "(GEOMETRIC_REPRESENTATION_ITEM()MAPPED_ITEM(#{map},#{target})REPRESENTATION_ITEM(''))"
        ));
        file.list_id(assembly.1, item);
        let text = file.text().replace(
            &format!(
                "#{}=(REPRESENTATION_RELATIONSHIP_PLACEHOLDER)('',(#{item}),#{MM});",
                assembly.1
            ),
            &format!(
                "#{}=(REPRESENTATION('',(#{item}),#{MM})SHAPE_REPRESENTATION());",
                assembly.1
            ),
        );
        let (structure, placements) = read(&text);
        assert_eq!(
            structure.usages()[0].placement,
            Some(Placement::MappedItem {
                mapped_item: item,
                representation_map: map,
                origin: Some(origin),
                target: Some(target),
            })
        );
        assert_eq!(
            placements.usage(index(structure, usage)),
            Ok(moved([0.0, 4.0, 0.0]))
        );

        // One with a camera image's partials is not read as a placement.
        let camera = text.replace(
            "(GEOMETRIC_REPRESENTATION_ITEM()MAPPED_ITEM",
            "(CAMERA_IMAGE()CAMERA_IMAGE_3D_WITH_SCALE()GEOMETRIC_REPRESENTATION_ITEM()MAPPED_ITEM",
        );
        let (structure, placements) = read(&camera);
        assert!(matches!(
            placements.usage(index(structure, usage)),
            Err(PlacementError::Unsupported { instance, .. }) if instance == item
        ));
    }

    #[test]
    fn errors_format_and_wrap() {
        let cases = [
            (
                PlacementError::Unplaced { usage: 7 },
                "usage #7: nothing places it (no context_dependent_shape_representation, and \
                 not as many mapped items as usages)",
            ),
            (
                PlacementError::Quantified {
                    usage: 7,
                    quantity: Some(4.0),
                },
                "usage #7: a quantity of 4, which one placement cannot place",
            ),
            (
                PlacementError::Divergent {
                    usage: 7,
                    instance: 9,
                    reason: DivergentReason::BothFramesPlaced,
                },
                "usage #7: #9: neither transform item is the identity, and ISO 10303 and Open \
                 CASCADE compose such a pair differently",
            ),
            (
                PlacementError::TooDeep { usage: 7 },
                "usage #7 lies more than 1024 levels below the root",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
            assert_eq!(crate::Error::from(error).to_string(), text);
        }
    }

    #[cfg(feature = "serde")]
    mod serialized {
        use super::*;
        use crate::testing::unique;

        #[test]
        fn occurrences_and_errors_have_unique_keys() {
            let geometry = GeometryError {
                instance: 3,
                entity: "DIRECTION".to_owned(),
                attribute: Some("direction_ratios"),
                kind: GeometryErrorKind::ZeroDirection,
            };
            let errors = [
                PlacementError::Unplaced { usage: 1 },
                PlacementError::Quantified {
                    usage: 1,
                    quantity: None,
                },
                PlacementError::NoTransformation {
                    usage: 1,
                    relationship: 2,
                },
                PlacementError::Unsupported {
                    usage: 1,
                    instance: 2,
                    entity: "X".to_owned(),
                },
                PlacementError::Inconsistent {
                    usage: 1,
                    instance: 2,
                    reason: SIDES,
                },
                PlacementError::Divergent {
                    usage: 1,
                    instance: 2,
                    reason: DivergentReason::DefaultedRefDirection,
                },
                PlacementError::Frame {
                    usage: 1,
                    error: geometry,
                },
                PlacementError::Cycle { usage: 1 },
                PlacementError::TooDeep { usage: 1 },
            ];
            for error in errors {
                let json = unique(&error);
                assert!(json.starts_with("{\"kind\":"), "{json}");
                unique(&Occurrence {
                    path: vec![0],
                    definition: 1,
                    representation: None,
                    transform: Err(error),
                });
            }
            assert_eq!(
                unique(&PlacementError::Divergent {
                    usage: 1,
                    instance: 2,
                    reason: DivergentReason::BothFramesPlaced,
                }),
                r#"{"kind":"divergent","usage":1,"instance":2,"reason":"both_frames_placed"}"#
            );
            let placed = unique(&Occurrence {
                path: vec![],
                definition: 0,
                representation: Some(5),
                transform: Ok(Rigid::IDENTITY),
            });
            assert!(
                placed.contains(r#""transform":{"ok":{"rotation""#),
                "{placed}"
            );
        }
    }
}
