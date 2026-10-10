//! B-rep topology the file states (ISO 10303-42): the shells, faces, loops,
//! edges and vertices of one solid, by `#id`.
//!
//! [`Solid::read`] reads a `manifold_solid_brep` or `brep_with_voids` and
//! everything it holds: its `closed_shell`s (an `oriented_closed_shell` as
//! its outer shell or a void), their `advanced_face`s, each face's
//! `face_bound`s and `face_outer_bound`s, their `edge_loop`s and
//! `vertex_loop`s, each loop's `oriented_edge`s, and the `edge_curve`s and
//! `vertex_point`s these name. Topology holds references and flags only:
//! a face's surface, an edge's curve and a vertex's point are `#id`s, read
//! with a [`geometry::Reader`](crate::geometry::Reader).
//!
//! Every flag is exposed exactly as written. Beyond the attributes, a few
//! values are read, not computed: the vertices an `oriented_edge` derives
//! ([`OrientedEdge::start`], [`OrientedEdge::end`]), the uses of each edge
//! ([`Solid::uses`]), a face's sense in its solid ([`Solid::face_sense`],
//! and the direction of each use of an edge ([`EdgeUse::forward`]), both
//! as the schema derives them through an `oriented_closed_shell`.
//!
//! A solid is read whole or not at all. Valid STEP this does not read (a
//! `faceted_brep`, a surface model, a `face_surface` that is not an
//! `advanced_face`, any complex instance) is
//! [`TopologyError::Unsupported`]; what contradicts the schema (a wrong
//! type, a wrong attribute count, a loop that does not close, an empty set,
//! a repeated oriented edge, a derived attribute written explicitly) is
//! [`TopologyError::Malformed`]; each names the innermost instance at fault.
//! Each instance is read once, so a read takes time linear in the size of
//! the file, however it is crafted.
//! Not checked: that the solid is manifold, that its geometry agrees with
//! its topology, and which bound of a face is its outer one when no
//! `face_outer_bound` says so.
//!
//! ```
//! use stepq::model::Graph;
//! use stepq::p21::parse;
//! use stepq::topology::{Loop, Solid};
//!
//! // A sphere: one face bounded by a loop of one vertex.
//! let src = b"ISO-10303-21;HEADER;FILE_SCHEMA(('AP214'));ENDSEC;DATA;
//! #1=MANIFOLD_SOLID_BREP('',#2);
//! #2=CLOSED_SHELL('',(#3));
//! #3=ADVANCED_FACE('',(#4),#20,.T.);
//! #4=FACE_BOUND('',#5,.T.);
//! #5=VERTEX_LOOP('',#6);
//! #6=VERTEX_POINT('',#21);
//! #20=SPHERICAL_SURFACE('',#22,5.);
//! #21=CARTESIAN_POINT('',(0.,0.,5.));
//! #22=AXIS2_PLACEMENT_3D('',#23,$,$);
//! #23=CARTESIAN_POINT('',(0.,0.,0.));
//! ENDSEC;END-ISO-10303-21;";
//!
//! let graph = Graph::new(parse(src)?)?;
//! let solid = Solid::read(&graph, 1)?;
//! assert_eq!(solid.faces().len(), 1);
//! assert_eq!(solid.faces()[0].surface, 20);
//! assert!(solid.face_sense(0)); // the outward normal is the surface's
//! assert!(solid.edges().is_empty());
//! assert!(matches!(
//!     solid.faces()[0].bounds[0].bound,
//!     Loop::VertexLoop { vertex: 0, .. }
//! ));
//! assert_eq!(solid.vertices()[0].point, 21);
//! # Ok::<(), stepq::Error>(())
//! ```

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::ops::Range;

use crate::model::Graph;
use crate::p21::{Exchange, Param, Record};
use crate::record;

/// One solid's topology, read by [`Solid::read`].
///
/// Shells, faces, edges and vertices are held in vectors and refer to one
/// another by index: [`Face::shell`] into [`shells`](Self::shells),
/// [`OrientedEdge::edge`] into [`edges`](Self::edges), [`Edge::start`] into
/// [`vertices`](Self::vertices). The outer shell comes first, then the
/// voids, each shell's faces in the order written; edges and vertices are
/// in the order first reached. Each element keeps the `#id` it was read
/// from. Serialized, a solid holds what the file states; the values read
/// from it ([`face_sense`](Self::face_sense), [`uses`](Self::uses)) are
/// methods.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Solid {
    /// The solid's `#id`.
    pub instance: u64,
    shells: Vec<Shell>,
    faces: Vec<Face>,
    edges: Vec<Edge>,
    vertices: Vec<Vertex>,
    /// `uses[use_offsets[e]..use_offsets[e + 1]]` are edge `e`'s uses.
    #[cfg_attr(feature = "serde", serde(skip))]
    use_offsets: Vec<usize>,
    #[cfg_attr(feature = "serde", serde(skip))]
    uses: Vec<EdgeUse>,
}

/// A shell of a [`Solid`]: its outer shell or a void.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Shell {
    /// The shell as the solid names it: a `closed_shell` or an
    /// `oriented_closed_shell`.
    pub instance: u64,
    /// The `closed_shell` that lists the faces: `instance` itself, or an
    /// oriented shell's `closed_shell_element`.
    pub closed_shell: u64,
    /// An `oriented_closed_shell`'s `orientation`; `None` for a
    /// `closed_shell`.
    pub orientation: Option<bool>,
    /// Its faces, as indices into [`Solid::faces`].
    pub faces: Range<usize>,
}

/// An `advanced_face`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Face {
    /// The face's `#id`.
    pub instance: u64,
    /// The shell that holds it, an index into [`Solid::shells`].
    pub shell: usize,
    /// `face_geometry`, the `#id` of its surface.
    pub surface: u64,
    /// `same_sense` as written: true if the face's normal is the surface's.
    /// Its sense in the solid is [`Solid::face_sense`].
    pub same_sense: bool,
    /// Its bounds, in the order written.
    pub bounds: Vec<FaceBound>,
}

/// A `face_bound` or `face_outer_bound`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FaceBound {
    /// The bound's `#id`.
    pub instance: u64,
    /// True for a `face_outer_bound`. False says only that the file does
    /// not state which bound is the outer one: a `face_bound` may be.
    pub outer: bool,
    /// The loop it bounds the face by.
    pub bound: Loop,
    /// `orientation` as written: true if the loop is traversed as stated.
    pub orientation: bool,
}

/// The loop of a [`FaceBound`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[non_exhaustive]
pub enum Loop {
    /// An `edge_loop`: oriented edges, each ending where the next starts.
    #[non_exhaustive]
    EdgeLoop {
        /// The loop's `#id`.
        instance: u64,
        /// Its oriented edges, in the order written.
        edges: Vec<OrientedEdge>,
    },
    /// A `vertex_loop`: one vertex, such as a sphere's pole.
    #[non_exhaustive]
    VertexLoop {
        /// The loop's `#id`.
        instance: u64,
        /// `loop_vertex`, an index into [`Solid::vertices`].
        vertex: usize,
    },
}

/// An `oriented_edge` of an edge loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct OrientedEdge {
    /// The oriented edge's `#id`.
    pub instance: u64,
    /// `edge_element`, an index into [`Solid::edges`].
    pub edge: usize,
    /// `orientation` as written: true if it runs from the edge's start to
    /// its end.
    pub orientation: bool,
    /// `edge_start` as the schema derives it: the edge's start if
    /// `orientation`, else its end. An index into [`Solid::vertices`].
    pub start: usize,
    /// `edge_end`, derived the same way.
    pub end: usize,
}

/// An `edge_curve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Edge {
    /// The edge's `#id`.
    pub instance: u64,
    /// `edge_start`, an index into [`Solid::vertices`].
    pub start: usize,
    /// `edge_end`, an index into [`Solid::vertices`].
    pub end: usize,
    /// `edge_geometry`, the `#id` of its curve (perhaps a `surface_curve` or
    /// `seam_curve`).
    pub curve: u64,
    /// `same_sense` as written: true if the edge runs along its curve's
    /// direction.
    pub same_sense: bool,
}

impl Edge {
    /// True if the edge starts and ends at one vertex instance, as a full
    /// circle does.
    pub fn is_closed(&self) -> bool {
        self.start == self.end
    }
}

/// A `vertex_point`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Vertex {
    /// The vertex's `#id`.
    pub instance: u64,
    /// `vertex_geometry`, the `#id` of its point.
    pub point: u64,
}

/// One use of an edge by an edge loop of a face, from [`Solid::uses`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct EdgeUse {
    /// The face, an index into [`Solid::faces`].
    pub face: usize,
    /// The bound, an index into that face's [`Face::bounds`].
    pub bound: usize,
    /// The oriented edge, an index into that bound's loop.
    pub index: usize,
    /// True if the face's boundary, as the solid holds the face, runs along
    /// the edge from its start to its end: the oriented edge's
    /// `orientation` equals its bound's, composed with the holding shell's
    /// [`orientation`](Shell::orientation). An `oriented_closed_shell` of
    /// orientation `.F.` reverses each face it holds and so each of its
    /// bounds (the schema's `conditional_reverse`), as
    /// [`Solid::face_sense`] does the face.
    pub forward: bool,
}

impl Solid {
    /// Reads the solid `#id`, a `manifold_solid_brep` or `brep_with_voids`,
    /// and all it holds.
    ///
    /// # Errors
    ///
    /// [`TopologyError::Missing`] if no instance has the `#id`, and any
    /// other [`TopologyError`] if the solid, or anything it holds, cannot be
    /// read as stated. Nothing of a solid is read if any part of it cannot
    /// be.
    pub fn read(graph: &Graph<'_>, id: u64) -> Result<Self, TopologyError> {
        let exchange = graph.exchange();
        let Some(instance) = exchange.get(id) else {
            return Err(TopologyError::Missing { instance: id });
        };
        if instance.is_complex() {
            return Err(TopologyError::Unsupported {
                instance: id,
                entity: record::entity(exchange, instance),
                expected: SOLID.expected,
            });
        }
        let mut walk = Walk {
            exchange,
            faces: Vec::new(),
            edges: Vec::new(),
            vertices: Vec::new(),
            listed_faces: HashSet::new(),
            bounding: HashMap::new(),
            oriented_index: HashMap::new(),
            edge_index: HashMap::new(),
            vertex_index: HashMap::new(),
        };
        let solid = walk.read(id, id, &SOLID)?;
        let outer = solid.reference(1, "outer")?;
        let mut shells = vec![walk.shell(id, outer, 0, &OUTER)?];
        if solid.name == "BREP_WITH_VOIDS" {
            for void in solid.set(2, "voids")? {
                let index = shells.len();
                shells.push(walk.shell(id, void, index, &VOIDS)?);
            }
        }
        let Walk {
            faces,
            edges,
            vertices,
            ..
        } = walk;
        let (use_offsets, uses) = edge_uses(&shells, &faces, edges.len());
        Ok(Self {
            instance: id,
            shells,
            faces,
            edges,
            vertices,
            use_offsets,
            uses,
        })
    }

    /// The outer shell.
    pub fn outer(&self) -> &Shell {
        &self.shells[0]
    }

    /// The voids of a `brep_with_voids`, in the order written; empty for a
    /// `manifold_solid_brep`.
    pub fn voids(&self) -> &[Shell] {
        &self.shells[1..]
    }

    /// Every shell: the outer one, then the voids.
    pub fn shells(&self) -> &[Shell] {
        &self.shells
    }

    /// Every face, shell by shell, in the order written.
    pub fn faces(&self) -> &[Face] {
        &self.faces
    }

    /// Every edge, in the order first reached.
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// Every vertex, in the order first reached.
    pub fn vertices(&self) -> &[Vertex] {
        &self.vertices
    }

    /// The sense of face `face` in the solid: true if the solid's outward
    /// normal there is its surface's normal.
    ///
    /// This is [`Face::same_sense`] composed with the holding shell's
    /// [`orientation`](Shell::orientation), as the schema derives the faces
    /// of an `oriented_closed_shell` (`conditional_reverse`). Use it, not
    /// `same_sense`, to tell a hole from a shaft, in the outer shell as in
    /// a void.
    ///
    /// # Panics
    ///
    /// Panics if `face` is out of range.
    pub fn face_sense(&self, face: usize) -> bool {
        let face = &self.faces[face];
        face.same_sense == self.shells[face.shell].orientation.unwrap_or(true)
    }

    /// Every use of edge `edge` by an edge loop, face by face in order. An
    /// edge of a closed manifold solid has two, in opposite directions.
    ///
    /// # Panics
    ///
    /// Panics if `edge` is out of range.
    pub fn uses(&self, edge: usize) -> &[EdgeUse] {
        &self.uses[self.use_offsets[edge]..self.use_offsets[edge + 1]]
    }

    /// True if edge `edge` is used at least twice, and only by one face: the
    /// seam of a closed surface such as a cylinder.
    ///
    /// # Panics
    ///
    /// Panics if `edge` is out of range.
    pub fn is_seam(&self, edge: usize) -> bool {
        match self.uses(edge) {
            [first, rest @ ..] if !rest.is_empty() => rest.iter().all(|u| u.face == first.face),
            _ => false,
        }
    }

    /// The faces that share an edge with face `face`, ascending, without
    /// `face` itself.
    ///
    /// # Panics
    ///
    /// Panics if `face` is out of range.
    pub fn neighbours(&self, face: usize) -> Vec<usize> {
        // Each edge once, so the work is bounded by the edges' uses.
        let mut edges: Vec<usize> = self.faces[face]
            .bounds
            .iter()
            .filter_map(|bound| match &bound.bound {
                Loop::EdgeLoop { edges, .. } => Some(edges),
                Loop::VertexLoop { .. } => None,
            })
            .flatten()
            .map(|oriented| oriented.edge)
            .collect();
        edges.sort_unstable();
        edges.dedup();
        let mut found: Vec<usize> = edges
            .into_iter()
            .flat_map(|edge| self.uses(edge).iter().map(|u| u.face))
            .filter(|&other| other != face)
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    /// The vertices of face `face`'s bounds, each once, in the order first
    /// met.
    ///
    /// # Panics
    ///
    /// Panics if `face` is out of range.
    pub fn face_vertices(&self, face: usize) -> Vec<usize> {
        let mut found = Vec::new();
        let mut seen = HashSet::new();
        for bound in &self.faces[face].bounds {
            match &bound.bound {
                Loop::EdgeLoop { edges, .. } => {
                    for oriented in edges {
                        if seen.insert(oriented.start) {
                            found.push(oriented.start);
                        }
                    }
                }
                Loop::VertexLoop { vertex, .. } => {
                    if seen.insert(*vertex) {
                        found.push(*vertex);
                    }
                }
            }
        }
        found
    }
}

/// The uses of each of `edges` edges by the loops of `faces`, held in
/// `shells`, as offsets into one vector of uses.
fn edge_uses(shells: &[Shell], faces: &[Face], edges: usize) -> (Vec<usize>, Vec<EdgeUse>) {
    let mut offsets = vec![0; edges + 1];
    let mut uses = Vec::new();
    for (face_index, face) in faces.iter().enumerate() {
        let as_written = shells[face.shell].orientation.unwrap_or(true);
        for (bound_index, bound) in face.bounds.iter().enumerate() {
            let Loop::EdgeLoop { edges, .. } = &bound.bound else {
                continue;
            };
            for (index, oriented) in edges.iter().enumerate() {
                offsets[oriented.edge + 1] += 1;
                uses.push((
                    oriented.edge,
                    EdgeUse {
                        face: face_index,
                        bound: bound_index,
                        index,
                        forward: (oriented.orientation == bound.orientation) == as_written,
                    },
                ));
            }
        }
    }
    for i in 1..offsets.len() {
        offsets[i] += offsets[i - 1];
    }
    // A stable sort by edge keeps each edge's uses in face order.
    uses.sort_by_key(|&(edge, _)| edge);
    (offsets, uses.into_iter().map(|(_, used)| used).collect())
}

/// Why a solid's topology cannot be read as stated.
///
/// Each variant names the innermost instance at fault, never the solid
/// unless it is the solid that is at fault.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[non_exhaustive]
pub enum TopologyError {
    /// No instance has the `#id` asked for.
    Missing {
        /// The `#id`.
        instance: u64,
    },
    /// An instance refers to an `#id` that is not defined.
    UnresolvedReference {
        /// The instance that holds the reference.
        from: u64,
        /// The `#id` it refers to.
        to: u64,
    },
    /// Valid STEP that is not read: a solid that is not a
    /// `manifold_solid_brep` or `brep_with_voids` (a `faceted_brep`, a
    /// surface model), a face that is not an `advanced_face`, a subtype of
    /// a topological entity read here that adds to it (an AP214
    /// `seam_edge`, a `tessellated_closed_shell`), a bound or loop that
    /// bounds more than one face (each is read once), or any complex
    /// instance.
    Unsupported {
        /// The instance not read.
        instance: u64,
        /// Its upper-cased entity types, joined with `+`.
        entity: String,
        /// What is read where it stands.
        expected: &'static str,
    },
    /// The file contradicts the schema: an instance of the wrong type for
    /// its attribute, a wrong number of attributes, an empty set, a flag
    /// other than `.T.` or `.F.`, a derived attribute written explicitly, a
    /// loop that does not close or that repeats an oriented edge, a face
    /// with two outer bounds or one bound listed twice, a face held twice by
    /// the solid, a void that
    /// is not an `oriented_closed_shell`, or what an `advanced_face`'s,
    /// `oriented_edge`'s or `oriented_closed_shell`'s rules forbid.
    Malformed {
        /// The instance at fault.
        instance: u64,
        /// Its upper-cased entity types, joined with `+`.
        entity: String,
        /// What is wrong.
        problem: String,
    },
}

impl fmt::Display for TopologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { instance } => write!(f, "#{instance} is not defined"),
            Self::UnresolvedReference { from, to } => {
                write!(f, "#{from} refers to #{to}, which is not defined")
            }
            Self::Unsupported {
                instance,
                entity,
                expected,
            } => write!(
                f,
                "#{instance} ({entity}) is not read; stepq reads only {expected} there"
            ),
            Self::Malformed {
                instance,
                entity,
                problem,
            } => write!(f, "#{instance} ({entity}): {problem}"),
        }
    }
}

/// What may stand in one attribute of the topology read here, and what
/// else is told apart from a wrong type.
struct Slot {
    /// The attribute, by its schema name.
    attribute: &'static str,
    /// What is read there, for messages.
    expected: &'static str,
    /// The entity types read there.
    read: &'static [&'static str],
    /// Valid subtypes that are not read: [`TopologyError::Unsupported`].
    unsupported: &'static [&'static str],
    /// Types the schema rules out there, and the rule that does:
    /// [`TopologyError::Malformed`] with that problem. Any other type is
    /// malformed as the wrong type.
    ruled_out: &'static [(&'static str, &'static str)],
    /// True if any type not read is unsupported, rather than malformed:
    /// the solid asked for may be any representation item.
    others_unsupported: bool,
}

const SOLID: Slot = Slot {
    attribute: "",
    expected: "manifold_solid_brep",
    read: &["MANIFOLD_SOLID_BREP", "BREP_WITH_VOIDS"],
    unsupported: &[],
    ruled_out: &[],
    others_unsupported: true,
};

const OUTER: Slot = Slot {
    attribute: "outer",
    expected: "closed_shell",
    read: &["CLOSED_SHELL", "ORIENTED_CLOSED_SHELL"],
    unsupported: &["TESSELLATED_CLOSED_SHELL"],
    ruled_out: &[],
    others_unsupported: false,
};

const VOIDS: Slot = Slot {
    attribute: "voids",
    expected: "oriented_closed_shell",
    read: &["ORIENTED_CLOSED_SHELL"],
    unsupported: &[],
    ruled_out: &[(
        "CLOSED_SHELL",
        "a void must be an oriented_closed_shell (brep_with_voids.voids)",
    )],
    others_unsupported: false,
};

const SHELL_ELEMENT: Slot = Slot {
    attribute: "closed_shell_element",
    expected: "closed_shell",
    read: &["CLOSED_SHELL"],
    unsupported: &["TESSELLATED_CLOSED_SHELL"],
    ruled_out: &[(
        "ORIENTED_CLOSED_SHELL",
        "an oriented_closed_shell's element must not be oriented itself",
    )],
    others_unsupported: false,
};

const FACES: Slot = Slot {
    attribute: "cfs_faces",
    expected: "advanced_face",
    read: &["ADVANCED_FACE"],
    unsupported: &[
        "FACE_SURFACE",
        "ORIENTED_FACE",
        "SUBFACE",
        "TESSELLATED_FACE_SURFACE",
    ],
    ruled_out: &[],
    others_unsupported: false,
};

const BOUNDS: Slot = Slot {
    attribute: "bounds",
    expected: "face_bound",
    read: &["FACE_BOUND", "FACE_OUTER_BOUND"],
    unsupported: &[],
    ruled_out: &[],
    others_unsupported: false,
};

const LOOP: Slot = Slot {
    attribute: "bound",
    expected: "edge_loop or vertex_loop",
    read: &["EDGE_LOOP", "VERTEX_LOOP"],
    unsupported: &["KINEMATIC_LOOP"],
    ruled_out: &[(
        "POLY_LOOP",
        "an advanced_face is bounded by edge and vertex loops only",
    )],
    others_unsupported: false,
};

const EDGE_LIST: Slot = Slot {
    attribute: "edge_list",
    expected: "oriented_edge",
    read: &["ORIENTED_EDGE"],
    unsupported: &["SEAM_EDGE", "ORIENTED_JOINT"],
    ruled_out: &[],
    others_unsupported: false,
};

const EDGE_ELEMENT: Slot = Slot {
    attribute: "edge_element",
    expected: "edge_curve",
    read: &["EDGE_CURVE"],
    unsupported: &["EDGE_BOUNDED_CURVE_WITH_LENGTH"],
    ruled_out: &[
        (
            "ORIENTED_EDGE",
            "an oriented_edge's element must not be oriented itself",
        ),
        ("SUBEDGE", "the edges of an advanced_face are edge_curves"),
    ],
    others_unsupported: false,
};

/// An `advanced_face`'s vertices are `vertex_point`s only (a rule of it
/// in every schema), so any other vertex is malformed, not merely
/// unsupported.
const VERTEX: Slot = Slot {
    attribute: "edge_start, edge_end or loop_vertex",
    expected: "vertex_point",
    read: &["VERTEX_POINT"],
    unsupported: &[],
    ruled_out: &[],
    others_unsupported: false,
};

/// The attribute count of each simple entity read here. Shells and solids
/// come from [`record::CONTAINERS`], which `units` walks too.
const COUNTS: &[(&str, usize)] = &[
    ("ADVANCED_FACE", 4),
    ("FACE_BOUND", 3),
    ("FACE_OUTER_BOUND", 3),
    ("EDGE_LOOP", 2),
    ("VERTEX_LOOP", 2),
    ("ORIENTED_EDGE", 5),
    ("EDGE_CURVE", 5),
    ("VERTEX_POINT", 2),
];

/// The record of one instance read, and its attributes.
struct Read<'e> {
    id: u64,
    /// Its entity type, upper case.
    name: &'static str,
    params: Vec<Param<'e>>,
}

impl Read<'_> {
    fn malformed(&self, problem: impl Into<String>) -> TopologyError {
        TopologyError::Malformed {
            instance: self.id,
            entity: self.name.to_owned(),
            problem: problem.into(),
        }
    }

    /// Attribute `index` as a reference.
    fn reference(&self, index: usize, attribute: &str) -> Result<u64, TopologyError> {
        self.params[index]
            .reference()
            .ok_or_else(|| self.malformed(format!("{attribute} is not a reference")))
    }

    /// Attribute `index` as a non-empty list of references, the `SET [1:?]`
    /// or `LIST [1:?]` every aggregate read here is.
    fn set(&self, index: usize, attribute: &str) -> Result<Vec<u64>, TopologyError> {
        let list = self.params[index]
            .list()
            .ok_or_else(|| self.malformed(format!("{attribute} is not a list")))?;
        let ids = list
            .map(|item| item.reference())
            .collect::<Option<Vec<u64>>>()
            .ok_or_else(|| {
                self.malformed(format!("{attribute} holds a value that is not a reference"))
            })?;
        if ids.is_empty() {
            return Err(self.malformed(format!("{attribute} is empty")));
        }
        Ok(ids)
    }

    /// Attribute `index` as a BOOLEAN: `.T.` or `.F.`.
    fn boolean(&self, index: usize, attribute: &str) -> Result<bool, TopologyError> {
        let value = match &self.params[index] {
            Param::Enumeration(literal) => literal.enumeration(),
            _ => None,
        };
        match value {
            Some(flag) if flag.eq_ignore_ascii_case("T") => Ok(true),
            Some(flag) if flag.eq_ignore_ascii_case("F") => Ok(false),
            _ => Err(self.malformed(format!("{attribute} is not .T. or .F."))),
        }
    }

    /// Requires attribute `index`, which a subtype redeclares as derived,
    /// to be written `*`.
    fn derived(&self, index: usize, attribute: &str) -> Result<(), TopologyError> {
        match self.params[index] {
            Param::Derived => Ok(()),
            _ => Err(self.malformed(format!("{attribute} is derived, so it must be written *"))),
        }
    }
}

struct Walk<'g, 'a> {
    exchange: &'g Exchange<'a>,
    faces: Vec<Face>,
    edges: Vec<Edge>,
    vertices: Vec<Vertex>,
    /// The faces listed by a shell so far.
    listed_faces: HashSet<u64>,
    /// The bounds and loops that bound a face so far, and that face.
    bounding: HashMap<u64, u64>,
    /// Each oriented edge read so far: its edge and orientation.
    oriented_index: HashMap<u64, (usize, bool)>,
    edge_index: HashMap<u64, usize>,
    vertex_index: HashMap<u64, usize>,
}

impl<'g> Walk<'g, '_> {
    /// Reads `#id`, which `#from` names in `slot`, if its type is one read
    /// there.
    fn read(&self, from: u64, id: u64, slot: &Slot) -> Result<Read<'g>, TopologyError> {
        let instance = self
            .exchange
            .get(id)
            .ok_or(TopologyError::UnresolvedReference { from, to: id })?;
        let entity = || record::entity(self.exchange, instance);
        let unsupported = || TopologyError::Unsupported {
            instance: id,
            entity: entity(),
            expected: slot.expected,
        };
        if instance.is_complex() {
            return Err(unsupported());
        }
        let record: Record<'g> = self
            .exchange
            .records(instance)
            .next()
            .ok_or_else(unsupported)?;
        let Some(&name) = slot.read.iter().find(|name| record.is(name)) else {
            if slot.others_unsupported || slot.unsupported.iter().any(|name| record.is(name)) {
                return Err(unsupported());
            }
            let problem = slot
                .ruled_out
                .iter()
                .find(|(name, _)| record.is(name))
                .map_or_else(
                    || {
                        format!(
                            "{} of #{from} must be of type {}",
                            slot.attribute, slot.expected
                        )
                    },
                    |(_, rule)| (*rule).to_owned(),
                );
            return Err(TopologyError::Malformed {
                instance: id,
                entity: entity(),
                problem,
            });
        };
        let count = record::container(record).map_or_else(
            || {
                COUNTS
                    .iter()
                    .find(|(entity, _)| *entity == name)
                    .map_or(0, |(_, count)| *count)
            },
            |container| container.params,
        );
        let read = Read {
            id,
            name,
            params: Vec::new(),
        };
        let params = record::exact_params(record, count).ok_or_else(|| {
            read.malformed(format!(
                "has {} attributes, where the schema gives {count}",
                record.params().count()
            ))
        })?;
        Ok(Read { params, ..read })
    }

    /// Reads the shell `#id`, which the solid `#solid` names in `slot`, as
    /// shell `index`, and its faces.
    fn shell(
        &mut self,
        solid: u64,
        id: u64,
        index: usize,
        slot: &Slot,
    ) -> Result<Shell, TopologyError> {
        let shell = self.read(solid, id, slot)?;
        let (closed, orientation) = if shell.name == "ORIENTED_CLOSED_SHELL" {
            shell.derived(1, "cfs_faces")?;
            let element = shell.reference(2, "closed_shell_element")?;
            let orientation = shell.boolean(3, "orientation")?;
            (self.read(id, element, &SHELL_ELEMENT)?, Some(orientation))
        } else {
            (shell, None)
        };
        let first = self.faces.len();
        for face in closed.set(1, "cfs_faces")? {
            if !self.listed_faces.insert(face) {
                let instance = self.exchange.get(face);
                return Err(TopologyError::Malformed {
                    instance: face,
                    entity: instance
                        .map(|i| record::entity(self.exchange, i))
                        .unwrap_or_default(),
                    problem: "the solid lists the face more than once".to_owned(),
                });
            }
            let face = self.face(closed.id, face, index)?;
            self.faces.push(face);
        }
        Ok(Shell {
            instance: id,
            closed_shell: closed.id,
            orientation,
            faces: first..self.faces.len(),
        })
    }

    fn face(&mut self, shell_id: u64, id: u64, shell: usize) -> Result<Face, TopologyError> {
        let face = self.read(shell_id, id, &FACES)?;
        let bound_ids = face.set(1, "bounds")?;
        let surface = face.reference(2, "face_geometry")?;
        let same_sense = face.boolean(3, "same_sense")?;
        let mut bounds = Vec::with_capacity(bound_ids.len());
        for bound_id in bound_ids {
            let bound = self.read(id, bound_id, &BOUNDS)?;
            self.bounds_once(&bound, id)?;
            let outer = bound.name == "FACE_OUTER_BOUND";
            if outer && bounds.iter().any(|b: &FaceBound| b.outer) {
                return Err(face.malformed("has more than one face_outer_bound"));
            }
            let loop_id = bound.reference(1, "bound")?;
            let orientation = bound.boolean(2, "orientation")?;
            let bound = self.bound_loop(id, bound_id, loop_id)?;
            bounds.push(FaceBound {
                instance: bound_id,
                outer,
                bound,
                orientation,
            });
        }
        Ok(Face {
            instance: id,
            shell,
            surface,
            same_sense,
            bounds,
        })
    }

    /// Requires a bound or loop to bound one face, `#face`, once. A face
    /// whose bounds list one bound twice contradicts its `SET`; faces that
    /// share a bound or loop are valid STEP, but not read: each is read
    /// once, which keeps the work linear in the file however it is crafted.
    fn bounds_once(&mut self, read: &Read<'_>, face: u64) -> Result<(), TopologyError> {
        match self.bounding.entry(read.id) {
            Entry::Vacant(entry) => {
                entry.insert(face);
                Ok(())
            }
            Entry::Occupied(entry)
                if *entry.get() == face
                    && matches!(read.name, "FACE_BOUND" | "FACE_OUTER_BOUND") =>
            {
                Err(read.malformed(format!(
                    "the bounds of #{face} list it more than once (a SET)"
                )))
            }
            Entry::Occupied(_) => Err(TopologyError::Unsupported {
                instance: read.id,
                entity: read.name.to_owned(),
                expected: "a bound or loop of one face",
            }),
        }
    }

    fn bound_loop(&mut self, face: u64, bound: u64, id: u64) -> Result<Loop, TopologyError> {
        let read = self.read(bound, id, &LOOP)?;
        self.bounds_once(&read, face)?;
        if read.name == "VERTEX_LOOP" {
            let vertex = read.reference(1, "loop_vertex")?;
            return Ok(Loop::VertexLoop {
                instance: id,
                vertex: self.vertex(id, vertex)?,
            });
        }
        let ids = read.set(1, "edge_list")?;
        let mut unique = HashSet::with_capacity(ids.len());
        let mut edges = Vec::with_capacity(ids.len());
        for oriented_id in ids {
            if !unique.insert(oriented_id) {
                return Err(read.malformed(format!(
                    "edge_list holds #{oriented_id} more than once (it is a LIST OF UNIQUE)"
                )));
            }
            let (edge, orientation) = self.oriented_edge(id, oriented_id)?;
            let Edge { start, end, .. } = self.edges[edge];
            let (start, end) = if orientation {
                (start, end)
            } else {
                (end, start)
            };
            edges.push(OrientedEdge {
                instance: oriented_id,
                edge,
                orientation,
                start,
                end,
            });
        }
        // path's WR1: each edge ends where the next starts, by instance;
        // edge_loop's WR1: the last ends where the first starts.
        for (k, oriented) in edges.iter().enumerate() {
            let next = &edges[(k + 1) % edges.len()];
            if oriented.end != next.start {
                return Err(read.malformed(format!(
                    "does not close: #{} ends at #{}, but #{} starts at #{}",
                    oriented.instance,
                    self.vertices[oriented.end].instance,
                    next.instance,
                    self.vertices[next.start].instance
                )));
            }
        }
        Ok(Loop::EdgeLoop {
            instance: id,
            edges,
        })
    }

    /// The edge and orientation of the oriented edge `#id`, read once
    /// however many loops list it.
    fn oriented_edge(&mut self, from: u64, id: u64) -> Result<(usize, bool), TopologyError> {
        if let Some(&read) = self.oriented_index.get(&id) {
            return Ok(read);
        }
        let oriented = self.read(from, id, &EDGE_LIST)?;
        oriented.derived(1, "edge_start")?;
        oriented.derived(2, "edge_end")?;
        let element = oriented.reference(3, "edge_element")?;
        let orientation = oriented.boolean(4, "orientation")?;
        let edge = self.edge(id, element)?;
        self.oriented_index.insert(id, (edge, orientation));
        Ok((edge, orientation))
    }

    fn edge(&mut self, from: u64, id: u64) -> Result<usize, TopologyError> {
        if let Some(&index) = self.edge_index.get(&id) {
            return Ok(index);
        }
        let edge = self.read(from, id, &EDGE_ELEMENT)?;
        let start = edge.reference(1, "edge_start")?;
        let end = edge.reference(2, "edge_end")?;
        let curve = edge.reference(3, "edge_geometry")?;
        let same_sense = edge.boolean(4, "same_sense")?;
        let start = self.vertex(id, start)?;
        let end = self.vertex(id, end)?;
        let index = self.edges.len();
        self.edges.push(Edge {
            instance: id,
            start,
            end,
            curve,
            same_sense,
        });
        self.edge_index.insert(id, index);
        Ok(index)
    }

    fn vertex(&mut self, from: u64, id: u64) -> Result<usize, TopologyError> {
        if let Some(&index) = self.vertex_index.get(&id) {
            return Ok(index);
        }
        let vertex = self.read(from, id, &VERTEX)?;
        let point = vertex.reference(1, "vertex_geometry")?;
        let index = self.vertices.len();
        self.vertices.push(Vertex {
            instance: id,
            point,
        });
        self.vertex_index.insert(id, index);
        Ok(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::parse;

    fn read(data: &str, solid: u64) -> Result<Solid, TopologyError> {
        let src = format!(
            "ISO-10303-21;HEADER;FILE_SCHEMA(('AP214'));ENDSEC;DATA;\n{data}\nENDSEC;END-ISO-10303-21;"
        );
        let src: &'static [u8] = Box::leak(src.into_bytes().into_boxed_slice());
        // build, not new: some cases refer to instances that are not defined.
        let graph = Graph::build(parse(src).unwrap());
        Solid::read(&graph, solid)
    }

    /// A pin of radius 3 and length 30 as Open CASCADE writes it: a
    /// cylinder bounded by its two circles and a seam line used twice, and
    /// two planar caps. Geometry is left out: topology names it only.
    const PIN: &str = "
#15=MANIFOLD_SOLID_BREP('',#16);
#16=CLOSED_SHELL('',(#17,#50,#59));
#17=ADVANCED_FACE('',(#18),#45,.T.);
#18=FACE_BOUND('',#19,.T.);
#19=EDGE_LOOP('',(#20,#30,#37,#38));
#20=ORIENTED_EDGE('',*,*,#21,.T.);
#21=EDGE_CURVE('',#22,#24,#26,.T.);
#22=VERTEX_POINT('',#23);
#24=VERTEX_POINT('',#25);
#30=ORIENTED_EDGE('',*,*,#31,.T.);
#31=EDGE_CURVE('',#24,#24,#32,.T.);
#37=ORIENTED_EDGE('',*,*,#21,.F.);
#38=ORIENTED_EDGE('',*,*,#39,.F.);
#39=EDGE_CURVE('',#22,#22,#40,.T.);
#50=ADVANCED_FACE('',(#51),#54,.F.);
#51=FACE_BOUND('',#52,.T.);
#52=EDGE_LOOP('',(#53));
#53=ORIENTED_EDGE('',*,*,#39,.T.);
#59=ADVANCED_FACE('',(#60),#63,.T.);
#60=FACE_BOUND('',#61,.F.);
#61=EDGE_LOOP('',(#62));
#62=ORIENTED_EDGE('',*,*,#31,.T.);";

    #[test]
    fn a_pin_with_a_seam() {
        let solid = read(PIN, 15).unwrap();
        assert_eq!(solid.instance, 15);
        assert_eq!(solid.outer().instance, 16);
        assert_eq!(solid.outer().closed_shell, 16);
        assert_eq!(solid.outer().orientation, None);
        assert_eq!(solid.voids(), []);
        let faces: Vec<u64> = solid.faces().iter().map(|f| f.instance).collect();
        assert_eq!(faces, [17, 50, 59]);
        assert_eq!(solid.faces()[0].surface, 45);
        let edges: Vec<u64> = solid.edges().iter().map(|e| e.instance).collect();
        assert_eq!(edges, [21, 31, 39]);
        let vertices: Vec<u64> = solid.vertices().iter().map(|v| v.instance).collect();
        assert_eq!(vertices, [22, 24]);

        // The seam is used twice by the cylinder, once each way.
        assert!(solid.is_seam(0));
        let seam: Vec<(usize, usize, bool)> = solid
            .uses(0)
            .iter()
            .map(|u| (u.face, u.index, u.forward))
            .collect();
        assert_eq!(seam, [(0, 0, true), (0, 2, false)]);
        // Each circle bounds the cylinder and a cap, in opposite directions,
        // the cap's bound reversed (#60) or its oriented edge (#38).
        for circle in [1, 2] {
            assert!(!solid.is_seam(circle));
            assert!(solid.edges()[circle].is_closed());
            let uses = solid.uses(circle);
            assert_eq!(uses.len(), 2);
            assert_ne!(uses[0].forward, uses[1].forward);
        }
        assert_eq!(solid.uses(1)[1].face, 2);
        assert!(
            !solid.uses(1)[1].forward,
            "#62 runs forward in a reversed bound"
        );

        // The oriented edges derive their vertices.
        let Loop::EdgeLoop { edges, .. } = &solid.faces()[0].bounds[0].bound else {
            panic!("an edge loop");
        };
        let ends: Vec<(usize, usize)> = edges.iter().map(|e| (e.start, e.end)).collect();
        assert_eq!(ends, [(0, 1), (1, 1), (1, 0), (0, 0)]);

        assert_eq!(solid.neighbours(0), [1, 2]);
        assert_eq!(solid.neighbours(1), [0]);
        assert_eq!(solid.face_vertices(0), [0, 1]);
        assert_eq!(solid.face_vertices(2), [1]);
        let senses: Vec<bool> = (0..3).map(|f| solid.face_sense(f)).collect();
        assert_eq!(senses, [true, false, true]);
        assert!(solid.faces().iter().all(|f| !f.bounds[0].outer));
    }

    /// The pin's faces with every sense reversed, under an
    /// `oriented_closed_shell` that reverses them back: `face_sense` still
    /// reads a shaft, where `same_sense` alone would read a hole.
    #[test]
    fn an_oriented_outer_shell_reverses_its_faces() {
        let reversed =
            PIN.replace(
                "#15=MANIFOLD_SOLID_BREP('',#16);",
                "#15=MANIFOLD_SOLID_BREP('',#900);",
            )
            .replace(
                "#17=ADVANCED_FACE('',(#18),#45,.T.)",
                "#17=ADVANCED_FACE('',(#18),#45,.F.)",
            )
            .replace(
                "#50=ADVANCED_FACE('',(#51),#54,.F.)",
                "#50=ADVANCED_FACE('',(#51),#54,.T.)",
            )
            .replace(
                "#59=ADVANCED_FACE('',(#60),#63,.T.)",
                "#59=ADVANCED_FACE('',(#60),#63,.F.)",
            ) + "#900=ORIENTED_CLOSED_SHELL('',*,#16,.F.);";
        let solid = read(&reversed, 15).unwrap();
        assert_eq!(solid.outer().instance, 900);
        assert_eq!(solid.outer().closed_shell, 16);
        assert_eq!(solid.outer().orientation, Some(false));
        assert!(!solid.faces()[0].same_sense);
        let senses: Vec<bool> = (0..3).map(|f| solid.face_sense(f)).collect();
        assert_eq!(senses, [true, false, true]);
    }

    /// A hollow solid as Creo and Open CASCADE write it: the void is a
    /// reversed `oriented_closed_shell`, so its cylinder reads as a hole.
    #[test]
    fn a_void_is_an_oriented_shell() {
        let inner = PIN.replace('#', "#1");
        let hollow = PIN.replace(
            "#15=MANIFOLD_SOLID_BREP('',#16);",
            "#15=BREP_WITH_VOIDS('',#16,(#2000));",
        ) + &inner.replace("#115=MANIFOLD_SOLID_BREP('',#116);", "")
            + "#2000=ORIENTED_CLOSED_SHELL('',*,#116,.F.);";
        let solid = read(&hollow, 15).unwrap();
        assert_eq!(solid.shells().len(), 2);
        let void = &solid.voids()[0];
        assert_eq!((void.instance, void.closed_shell), (2000, 116));
        assert_eq!(void.faces, 3..6);
        assert_eq!(solid.faces()[3].shell, 1);
        assert!(solid.face_sense(0), "the outer cylinder is a shaft");
        assert!(!solid.face_sense(3), "the void's cylinder is a hole");
        // The void's faces, reversed in the solid, run their bounds the
        // other way: its seam's first use is backward, the outer one's
        // forward.
        assert!(solid.uses(0)[0].forward);
        assert_eq!(solid.edges()[3].instance, 121);
        assert!(!solid.uses(3)[0].forward);
        assert_eq!(solid.edges().len(), 6);

        let plain = hollow.replace("(#2000)", "(#116)");
        assert_eq!(
            read(&plain, 15),
            Err(TopologyError::Malformed {
                instance: 116,
                entity: "CLOSED_SHELL".to_owned(),
                problem: "a void must be an oriented_closed_shell (brep_with_voids.voids)"
                    .to_owned(),
            })
        );
    }

    /// A sphere, bounded by one vertex; and a face bounded by two loops,
    /// neither stated to be outer, as Unigraphics writes a plate with a
    /// hole.
    #[test]
    fn vertex_loops_and_unstated_outer_bounds() {
        let sphere = "#1=MANIFOLD_SOLID_BREP('',#2);#2=CLOSED_SHELL('',(#3));\
             #3=ADVANCED_FACE('',(#4),#20,.T.);#4=FACE_BOUND('',#5,.T.);\
             #5=VERTEX_LOOP('',#6);#6=VERTEX_POINT('',#21);";
        let solid = read(sphere, 1).unwrap();
        assert_eq!(solid.edges(), []);
        assert_eq!(solid.vertices().len(), 1);
        assert_eq!(solid.face_vertices(0), [0]);
        assert_eq!(solid.neighbours(0), Vec::<usize>::new());

        let two_bounds = PIN.replace(
            "#59=ADVANCED_FACE('',(#60),#63,.T.);",
            "#59=ADVANCED_FACE('',(#60,#70),#63,.T.);#70=FACE_BOUND('',#71,.T.);\
             #71=VERTEX_LOOP('',#72);#72=VERTEX_POINT('',#73);",
        );
        let solid = read(&two_bounds, 15).unwrap();
        let bounds = &solid.faces()[2].bounds;
        assert_eq!(bounds.len(), 2);
        assert!(bounds.iter().all(|b| !b.outer));
        assert_eq!(solid.face_vertices(2), [1, 2]);

        let outer = two_bounds.replace("#70=FACE_BOUND", "#70=FACE_OUTER_BOUND");
        assert!(read(&outer, 15).unwrap().faces()[2].bounds[1].outer);
        let both = outer.replace("#60=FACE_BOUND", "#60=FACE_OUTER_BOUND");
        assert_eq!(
            read(&both, 15),
            Err(TopologyError::Malformed {
                instance: 59,
                entity: "ADVANCED_FACE".to_owned(),
                problem: "has more than one face_outer_bound".to_owned(),
            })
        );
    }

    fn unsupported(instance: u64, entity: &str, expected: &'static str) -> TopologyError {
        TopologyError::Unsupported {
            instance,
            entity: entity.to_owned(),
            expected,
        }
    }

    fn malformed(instance: u64, entity: &str, problem: &str) -> TopologyError {
        TopologyError::Malformed {
            instance,
            entity: entity.to_owned(),
            problem: problem.to_owned(),
        }
    }

    fn refused(cases: Vec<(String, u64, TopologyError)>) {
        for (data, solid, expected) in cases {
            assert_eq!(read(&data, solid), Err(expected), "{data}");
        }
    }

    /// Valid STEP that is not read is refused as unsupported, at the
    /// instance not read.
    #[test]
    fn unsupported_types_are_named() {
        refused(vec![
            // Faces that share a loop or a bound: valid, but each is read once.
            (
                PIN.replace("#60=FACE_BOUND('',#61,.F.);", "#60=FACE_BOUND('',#52,.F.);"),
                15,
                unsupported(52, "EDGE_LOOP", "a bound or loop of one face"),
            ),
            (
                PIN.replace(
                    "#59=ADVANCED_FACE('',(#60),#63,.T.);",
                    "#59=ADVANCED_FACE('',(#51),#63,.T.);",
                ),
                15,
                unsupported(51, "FACE_BOUND", "a bound or loop of one face"),
            ),
            (
                PIN.replace("MANIFOLD_SOLID_BREP", "FACETED_BREP"),
                15,
                unsupported(15, "FACETED_BREP", "manifold_solid_brep"),
            ),
            (
                format!("{PIN}#1=SHELL_BASED_SURFACE_MODEL('',(#16));"),
                1,
                unsupported(1, "SHELL_BASED_SURFACE_MODEL", "manifold_solid_brep"),
            ),
            (
                PIN.replace("#17=ADVANCED_FACE", "#17=FACE_SURFACE"),
                15,
                unsupported(17, "FACE_SURFACE", "advanced_face"),
            ),
            (
                format!(
                    "{}#81=ORIENTED_FACE('',*,#17,.F.);",
                    PIN.replace("(#17,#50,#59)", "(#81,#50,#59)")
                ),
                15,
                unsupported(81, "ORIENTED_FACE", "advanced_face"),
            ),
            (
                PIN.replace(
                    "#22=VERTEX_POINT('',#23);",
                    "#22=(VERTEX_POINT('',#23)TOPOLOGICAL_REPRESENTATION_ITEM());",
                ),
                15,
                unsupported(
                    22,
                    "VERTEX_POINT+TOPOLOGICAL_REPRESENTATION_ITEM",
                    "vertex_point",
                ),
            ),
            (
                PIN.replace(
                    "#53=ORIENTED_EDGE('',*,*,#39,.T.);",
                    "#53=SEAM_EDGE('',*,*,#39,.T.,#9);",
                ),
                15,
                unsupported(53, "SEAM_EDGE", "oriented_edge"),
            ),
        ]);
    }

    /// A type the schema rules out where it stands is refused as malformed,
    /// at that instance.
    #[test]
    fn malformed_types_are_named() {
        refused(vec![
            (
                PIN.replace("#52=EDGE_LOOP('',(#53));", "#52=POLY_LOOP('',(#23));"),
                15,
                malformed(
                    52,
                    "POLY_LOOP",
                    "an advanced_face is bounded by edge and vertex loops only",
                ),
            ),
            (
                PIN.replace(
                    "#21=EDGE_CURVE('',#22,#24,#26,.T.);",
                    "#21=SUBEDGE('',#22,#24,#39);",
                ),
                15,
                malformed(
                    21,
                    "SUBEDGE",
                    "the edges of an advanced_face are edge_curves",
                ),
            ),
            (
                PIN.replace(
                    "#53=ORIENTED_EDGE('',*,*,#39,.T.);",
                    "#53=ORIENTED_EDGE('',*,*,#38,.T.);",
                ),
                15,
                malformed(
                    38,
                    "ORIENTED_EDGE",
                    "an oriented_edge's element must not be oriented itself",
                ),
            ),
            (
                format!(
                    "{}#900=ORIENTED_CLOSED_SHELL('',*,#901,.F.);\
                     #901=ORIENTED_CLOSED_SHELL('',*,#16,.F.);",
                    PIN.replace(
                        "MANIFOLD_SOLID_BREP('',#16)",
                        "MANIFOLD_SOLID_BREP('',#900)"
                    )
                ),
                15,
                malformed(
                    901,
                    "ORIENTED_CLOSED_SHELL",
                    "an oriented_closed_shell's element must not be oriented itself",
                ),
            ),
            (
                PIN.replace("#22=VERTEX_POINT('',#23);", "#22=VERTEX('');"),
                15,
                malformed(
                    22,
                    "VERTEX",
                    "edge_start, edge_end or loop_vertex of #21 must be of type vertex_point",
                ),
            ),
            (
                PIN.replace("#18=FACE_BOUND('',#19,.T.);", "#18=EDGE_LOOP('',(#20));"),
                15,
                malformed(18, "EDGE_LOOP", "bounds of #17 must be of type face_bound"),
            ),
        ]);
    }

    /// A set, flag, attribute count or loop that contradicts the schema is
    /// refused as malformed at the instance that states it; an undefined
    /// reference at the instance that holds it.
    #[test]
    fn malformed_structure_is_named() {
        refused(vec![
            (
                PIN.replace(
                    "#16=CLOSED_SHELL('',(#17,#50,#59));",
                    "#16=CLOSED_SHELL('',());",
                ),
                15,
                malformed(16, "CLOSED_SHELL", "cfs_faces is empty"),
            ),
            (
                PIN.replace("(#17,#50,#59)", "(#17,#50,#59,#50)"),
                15,
                malformed(
                    50,
                    "ADVANCED_FACE",
                    "the solid lists the face more than once",
                ),
            ),
            (
                PIN.replace("#45,.T.)", "#45,.U.)"),
                15,
                malformed(17, "ADVANCED_FACE", "same_sense is not .T. or .F."),
            ),
            (
                PIN.replace(
                    "#20=ORIENTED_EDGE('',*,*,#21,.T.);",
                    "#20=ORIENTED_EDGE('',#22,*,#21,.T.);",
                ),
                15,
                malformed(
                    20,
                    "ORIENTED_EDGE",
                    "edge_start is derived, so it must be written *",
                ),
            ),
            (
                PIN.replace(
                    "#21=EDGE_CURVE('',#22,#24,#26,.T.);",
                    "#21=EDGE_CURVE('',#22,#24,#26);",
                ),
                15,
                malformed(
                    21,
                    "EDGE_CURVE",
                    "has 4 attributes, where the schema gives 5",
                ),
            ),
            (
                PIN.replace("(#20,#30,#37,#38)", "(#20,#30,#37,#38,#20)"),
                15,
                malformed(
                    19,
                    "EDGE_LOOP",
                    "edge_list holds #20 more than once (it is a LIST OF UNIQUE)",
                ),
            ),
            (
                PIN.replace("(#20,#30,#37,#38)", "(#20,#37,#30,#38)"),
                15,
                malformed(
                    19,
                    "EDGE_LOOP",
                    "does not close: #37 ends at #22, but #30 starts at #24",
                ),
            ),
            (
                PIN.replace("(#18),#45", "(#18,#18),#45"),
                15,
                malformed(
                    18,
                    "FACE_BOUND",
                    "the bounds of #17 list it more than once (a SET)",
                ),
            ),
            // Not defined.
            (
                PIN.replace("#39=EDGE_CURVE('',#22,#22,#40,.T.);", ""),
                15,
                TopologyError::UnresolvedReference { from: 38, to: 39 },
            ),
            (PIN.to_owned(), 99, TopologyError::Missing { instance: 99 }),
        ]);
    }

    /// Loops close by vertex instance, wherever they start: a rotation of
    /// the pin's loop closes, and a vertex at the same point that is another
    /// instance does not.
    #[test]
    fn loops_close_by_instance() {
        let swapped = PIN.replace("(#20,#30,#37,#38)", "(#37,#38,#20,#30)");
        assert!(read(&swapped, 15).is_ok());
        let split = PIN
            .replace(
                "#31=EDGE_CURVE('',#24,#24,#32,.T.);",
                "#31=EDGE_CURVE('',#24,#26,#32,.T.);",
            )
            .replace(
                "#24=VERTEX_POINT('',#25);",
                "#24=VERTEX_POINT('',#25);#26=VERTEX_POINT('',#25);",
            );
        assert!(matches!(
            read(&split, 15),
            Err(TopologyError::Malformed { instance: 19, .. })
        ));
    }

    /// The attributes of each entity read here, by name, and whether a
    /// subtype redeclares each as derived.
    const LAYOUTS: &[(&str, &[(&str, bool)])] = &[
        ("MANIFOLD_SOLID_BREP", &[("name", false), ("outer", false)]),
        (
            "BREP_WITH_VOIDS",
            &[("name", false), ("outer", false), ("voids", false)],
        ),
        ("CLOSED_SHELL", &[("name", false), ("cfs_faces", false)]),
        (
            "ORIENTED_CLOSED_SHELL",
            &[
                ("name", false),
                ("cfs_faces", true),
                ("closed_shell_element", false),
                ("orientation", false),
            ],
        ),
        (
            "ADVANCED_FACE",
            &[
                ("name", false),
                ("bounds", false),
                ("face_geometry", false),
                ("same_sense", false),
            ],
        ),
        (
            "FACE_BOUND",
            &[("name", false), ("bound", false), ("orientation", false)],
        ),
        (
            "FACE_OUTER_BOUND",
            &[("name", false), ("bound", false), ("orientation", false)],
        ),
        ("EDGE_LOOP", &[("name", false), ("edge_list", false)]),
        ("VERTEX_LOOP", &[("name", false), ("loop_vertex", false)]),
        (
            "ORIENTED_EDGE",
            &[
                ("name", false),
                ("edge_start", true),
                ("edge_end", true),
                ("edge_element", false),
                ("orientation", false),
            ],
        ),
        (
            "EDGE_CURVE",
            &[
                ("name", false),
                ("edge_start", false),
                ("edge_end", false),
                ("edge_geometry", false),
                ("same_sense", false),
            ],
        ),
        (
            "VERTEX_POINT",
            &[("name", false), ("vertex_geometry", false)],
        ),
    ];

    /// Every entity read here has the attributes this reads, at the
    /// positions read, in every schema fetched (`tools/fetch-schemas.sh`;
    /// skipped where none is).
    #[test]
    fn attribute_positions_match_the_schemas() {
        use crate::express::Schema;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/schemas");
        let mut checked = 0;
        let mut in_some_schema = HashSet::new();
        for file in [
            "ap242e4_mim_lf.exp",
            "ap214e3.exp",
            "ap203e2_mim_lf.exp",
            "ap203.exp",
        ] {
            let Ok(src) = std::fs::read(dir.join(file)) else {
                continue;
            };
            let schema = Schema::parse(&src).unwrap();
            for (entity, layout) in LAYOUTS {
                let slots = schema.slots(entity).unwrap();
                let found: Vec<(String, bool)> = slots
                    .iter()
                    .map(|slot| (slot.name.to_ascii_lowercase(), slot.derived))
                    .collect();
                let layout: Vec<(String, bool)> = layout
                    .iter()
                    .map(|&(name, derived)| (name.to_owned(), derived))
                    .collect();
                assert_eq!(found, layout, "{file}: {entity}");
                let count = record::CONTAINERS
                    .iter()
                    .find(|c| c.entity == *entity)
                    .map(|c| c.params)
                    .or_else(|| COUNTS.iter().find(|(e, _)| e == entity).map(|(_, n)| *n));
                assert_eq!(count, Some(layout.len()), "{entity}");
            }
            // Every type a slot reads is a type of each schema.
            for slot in SLOTS {
                for name in slot.read {
                    assert!(schema.entity(name).is_some(), "{file}: {name}");
                }
            }
            for slot in SLOTS {
                let told_apart = slot
                    .unsupported
                    .iter()
                    .chain(slot.ruled_out.iter().map(|(n, _)| n));
                for name in told_apart {
                    if schema.entity(name).is_some() {
                        in_some_schema.insert(*name);
                    }
                }
            }
            checked += 1;
        }
        if checked == 0 {
            eprintln!(
                "skipping: no schema under {} (run tools/fetch-schemas.sh)",
                dir.display()
            );
            return;
        }
        // Every type a slot refuses or tells apart is a type of some schema
        // (AP214's seam_edge is in no other): a misspelt name would fall
        // through to the generic message.
        for slot in SLOTS {
            let told_apart = slot
                .unsupported
                .iter()
                .chain(slot.ruled_out.iter().map(|(n, _)| n));
            for name in told_apart {
                assert!(in_some_schema.contains(name), "{name} is in no schema");
            }
        }
    }

    const SLOTS: [&Slot; 10] = [
        &SOLID,
        &OUTER,
        &VOIDS,
        &SHELL_ELEMENT,
        &FACES,
        &BOUNDS,
        &LOOP,
        &EDGE_LIST,
        &EDGE_ELEMENT,
        &VERTEX,
    ];

    #[test]
    fn errors_format_and_wrap() {
        let cases = [
            (TopologyError::Missing { instance: 7 }, "#7 is not defined"),
            (
                TopologyError::UnresolvedReference { from: 7, to: 9 },
                "#7 refers to #9, which is not defined",
            ),
            (
                TopologyError::Unsupported {
                    instance: 7,
                    entity: "FACE_SURFACE".to_owned(),
                    expected: "advanced_face",
                },
                "#7 (FACE_SURFACE) is not read; stepq reads only advanced_face there",
            ),
            (
                TopologyError::Malformed {
                    instance: 7,
                    entity: "EDGE_LOOP".to_owned(),
                    problem: "edge_list is empty".to_owned(),
                },
                "#7 (EDGE_LOOP): edge_list is empty",
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
        fn solids_and_errors_have_unique_keys() {
            let solid = read(PIN, 15).unwrap();
            let json = unique(&solid);
            assert!(json.contains("\"kind\":\"edge_loop\""), "{json}");
            let sphere = read(
                "#1=MANIFOLD_SOLID_BREP('',#2);#2=CLOSED_SHELL('',(#3));\
                 #3=ADVANCED_FACE('',(#4),#20,.T.);#4=FACE_BOUND('',#5,.T.);\
                 #5=VERTEX_LOOP('',#6);#6=VERTEX_POINT('',#21);",
                1,
            )
            .unwrap();
            assert!(unique(&sphere).contains("\"kind\":\"vertex_loop\""));
            let errors = [
                TopologyError::Missing { instance: 1 },
                TopologyError::UnresolvedReference { from: 1, to: 2 },
                TopologyError::Unsupported {
                    instance: 1,
                    entity: "X".to_owned(),
                    expected: "advanced_face",
                },
                TopologyError::Malformed {
                    instance: 1,
                    entity: "X".to_owned(),
                    problem: "p".to_owned(),
                },
            ];
            for error in errors {
                let json = unique(&error);
                assert!(json.starts_with("{\"kind\":"), "{json}");
            }
        }
    }
}
