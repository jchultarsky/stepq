//! Solid topology read from the fetched fixtures (`stepq::topology`).
//!
//! Fixtures are downloaded by `tools/fetch-fixtures.sh` and are not in git,
//! so on a fresh clone these tests find nothing and pass; with
//! `STEPQ_REQUIRE_FIXTURES=1` a missing core fixture fails them. Files over
//! 16 MB are read only with `STEPQ_LARGE_FIXTURES=1`.

mod common;

use std::fs;

use common::{fixture, include_large, is_core, require_core_set, step_files};
use stepq::model::Graph;
use stepq::p21::{Exchange, Instance, parse};
use stepq::topology::{Loop, Solid, TopologyError};
use stepq::units::ContextUnits;

fn graph(src: &[u8]) -> Graph<'_> {
    Graph::new(parse(src).expect("fixture parses")).expect("fixture resolves")
}

/// The solids topology reads.
const SOLIDS: &[&str] = &["MANIFOLD_SOLID_BREP", "BREP_WITH_VOIDS"];
/// Solid models and surface models it does not.
const NOT_READ: &[&str] = &[
    "FACETED_BREP",
    "SHELL_BASED_SURFACE_MODEL",
    "FACE_BASED_SURFACE_MODEL",
];

fn is_simple(exchange: &Exchange<'_>, instance: &Instance, names: &[&str]) -> bool {
    !instance.is_complex()
        && exchange
            .records(instance)
            .next()
            .is_some_and(|record| names.iter().any(|name| record.is(name)))
}

/// Solids, faces, edges, vertices and seams, summed over a file's solids.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Totals {
    solids: usize,
    faces: usize,
    edges: usize,
    vertices: usize,
    seams: usize,
}

impl Totals {
    fn add(&mut self, solid: &Solid) {
        self.solids += 1;
        self.faces += solid.faces().len();
        self.edges += solid.edges().len();
        self.vertices += solid.vertices().len();
        self.seams += (0..solid.edges().len())
            .filter(|&e| solid.is_seam(e))
            .count();
    }
}

/// What a file's solids and models read as.
#[derive(Debug, Default)]
struct Read {
    totals: Totals,
    /// Refusals of a `manifold_solid_brep` or `brep_with_voids`.
    refused: Vec<(u64, TopologyError)>,
    /// Solid and surface models not read, each refused as unsupported.
    unsupported: usize,
}

/// Reads every solid and model of one file, and checks each solid read:
/// every edge used twice, in opposite directions, a seam by one face only,
/// each loop chaining its oriented edges by vertex, and each face's units
/// readable through the representations that hold it.
fn read(src: &[u8]) -> Read {
    let graph = graph(src);
    let exchange = graph.exchange();
    let mut read = Read::default();
    for instance in exchange.instances() {
        let id = instance.id;
        if is_simple(exchange, instance, NOT_READ) {
            let error = Solid::read(&graph, id).unwrap_err();
            assert!(
                matches!(error, TopologyError::Unsupported { instance, .. } if instance == id),
                "#{id}: {error}"
            );
            read.unsupported += 1;
            continue;
        }
        if !is_simple(exchange, instance, SOLIDS) {
            continue;
        }
        let solid = match Solid::read(&graph, id) {
            Ok(solid) => solid,
            Err(error) => {
                read.refused.push((id, error));
                continue;
            }
        };
        check(&graph, &solid);
        read.totals.add(&solid);
    }
    read
}

fn check(graph: &Graph<'_>, solid: &Solid) {
    let id = solid.instance;
    for (index, edge) in solid.edges().iter().enumerate() {
        let uses = solid.uses(index);
        assert_eq!(uses.len(), 2, "#{id}: edge #{}", edge.instance);
        assert_ne!(
            uses[0].forward, uses[1].forward,
            "#{id}: edge #{}",
            edge.instance
        );
        if solid.is_seam(index) {
            assert_eq!(uses[0].face, uses[1].face);
        }
    }
    for (index, face) in solid.faces().iter().enumerate() {
        for bound in &face.bounds {
            if let Loop::EdgeLoop { edges, .. } = &bound.bound {
                for (k, oriented) in edges.iter().enumerate() {
                    let next = &edges[(k + 1) % edges.len()];
                    assert_eq!(oriented.end, next.start, "#{id}: face #{}", face.instance);
                }
            }
        }
        assert!(
            solid.neighbours(index).iter().all(|&other| other != index),
            "#{id}"
        );
        if let Err(error) = ContextUnits::of_item(graph, face.instance) {
            panic!("#{id}: face #{}: {error}", face.instance);
        }
    }
}

/// Totals pinned per file: the AS1 assembly from five exporters (the
/// faceless edges Unigraphics and Mechanical Desktop leave out give 39
/// faces, not 53), a clevis, and NIST's STC 9, the only core file with
/// seams (33).
#[test]
fn totals_per_file() {
    let pinned = [
        ("steptools/as1-ac-214.stp", AS1),
        ("steptools/as1-ec-214.stp", AS1),
        ("steptools/as1-tc-214.stp", AS1),
        ("nist-edm/as1_pe.stp", AS1),
        ("steptools/as1-md-214.stp", AS1_SPLIT),
        ("steptools/as1-ug-214.stp", AS1_SPLIT),
        (
            "nist-edm/clevis21.stp",
            Totals {
                solids: 1,
                faces: 48,
                edges: 78,
                vertices: 62,
                seams: 0,
            },
        ),
        (
            "nist/NIST-PMI-STEP-Files/nist_stc_09_asme1_ap242-e3.stp",
            Totals {
                solids: 1,
                faces: 125,
                edges: 392,
                vertices: 271,
                seams: 33,
            },
        ),
    ];
    for (file, expected) in pinned {
        let Some(src) = fixture(file) else {
            continue;
        };
        let read = read(&src);
        assert!(read.refused.is_empty(), "{file}: {:?}", read.refused);
        assert_eq!(read.totals, expected, "{file}");
    }
}

const AS1: Totals = Totals {
    solids: 5,
    faces: 53,
    edges: 126,
    vertices: 84,
    seams: 0,
};

const AS1_SPLIT: Totals = Totals {
    solids: 5,
    faces: 39,
    edges: 70,
    vertices: 56,
    seams: 0,
};

/// Every solid of every fixture reads and passes the checks of [`read`];
/// every surface model and faceted B-rep is refused as unsupported. The one
/// solid refused is a Creo chassis part whose loop #1200882 does not close.
#[test]
fn every_solid_reads() {
    let files = step_files();
    require_core_set(&files);
    let (mut core, mut all) = (Totals::default(), Totals::default());
    let (mut core_not_read, mut all_not_read) = (0, 0);
    let mut refused = Vec::new();
    for path in &files {
        let src = fs::read(path).expect("fixture is readable");
        let read = read(&src);
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        refused.extend(
            read.refused
                .into_iter()
                .map(|(id, e)| (name.clone(), id, e)),
        );
        for (totals, not_read, add) in [
            (&mut all, &mut all_not_read, true),
            (&mut core, &mut core_not_read, is_core(path)),
        ] {
            if add {
                totals.solids += read.totals.solids;
                totals.faces += read.totals.faces;
                totals.edges += read.totals.edges;
                totals.vertices += read.totals.vertices;
                totals.seams += read.totals.seams;
                *not_read += read.unsupported;
            }
        }
    }
    let chassis: Vec<(&str, u64, u64)> = refused
        .iter()
        .map(|(file, solid, error)| {
            let TopologyError::Malformed { instance, .. } = error else {
                panic!("{file}: #{solid}: {error}");
            };
            (file.as_str(), *solid, *instance)
        })
        .collect();
    assert!(
        chassis.is_empty() || chassis == [("001-e2000_assy_master_asm.stp", CHASSIS.0, CHASSIS.1)],
        "{refused:?}"
    );
    if files.iter().filter(|path| is_core(path)).count() == common::CORE_FILES {
        assert_eq!((core, core_not_read), CORE, "the core fixtures");
    }
    if include_large() && files.len() == 55 {
        assert_eq!((all, all_not_read), ALL, "all 55 fixtures");
        assert_eq!(chassis.len(), 1);
    }
}

/// The Creo chassis part refused, and its loop that does not close.
const CHASSIS: (u64, u64) = (1_201_538, 1_200_882);

const CORE: (Totals, usize) = (
    Totals {
        solids: 118,
        faces: 9_869,
        edges: 23_582,
        vertices: 15_572,
        seams: 33,
    },
    31,
);

const ALL: (Totals, usize) = (
    Totals {
        solids: 1_260,
        faces: 257_358,
        edges: 715_015,
        vertices: 463_162,
        seams: 33,
    },
    87,
);
