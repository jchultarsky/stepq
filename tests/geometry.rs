//! Geometry values read from the fetched fixtures (`stepq::geometry`).
//!
//! Fixtures are downloaded by `tools/fetch-fixtures.sh` and are not in git,
//! so on a fresh clone these tests find nothing and pass; with
//! `STEPQ_REQUIRE_FIXTURES=1` a missing core fixture fails them. Files over
//! 16 MB are read only with `STEPQ_LARGE_FIXTURES=1`.
//!
//! Faces and edges are found as `examples/geometry.rs` finds them, walking
//! forward from each representation's items, but from every representation,
//! supplemental geometry included. Each face is read in the units of the
//! representations that hold it (`Reader::for_item`), each edge with the
//! reader of a face it bounds.

mod common;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::f64::consts::PI;
use std::fs;

use common::{fixture, include_large, is_core, require_core_set, step_files};
use stepq::geometry::{
    Curve, MasterRepresentation, Reader, Surface, SurfaceCurve, SurfaceCurveKind,
};
use stepq::model::Graph;
use stepq::p21::{Exchange, Instance, Param, parse};

fn graph(src: &[u8]) -> Graph<'_> {
    Graph::new(parse(src).expect("fixture parses")).expect("fixture resolves")
}

/// Types whose name ends in REPRESENTATION but which are not
/// representations.
const NOT_REPRESENTATIONS: &[&str] = &[
    "SHAPE_DEFINITION_REPRESENTATION",
    "PROPERTY_DEFINITION_REPRESENTATION",
    "DIMENSIONAL_CHARACTERISTIC_REPRESENTATION",
    "CHARACTERIZED_ITEM_WITHIN_REPRESENTATION",
    "PRESENTED_ITEM_REPRESENTATION",
];

fn names(exchange: &Exchange<'_>, instance: &Instance) -> Vec<String> {
    exchange
        .records(instance)
        .map(|record| String::from_utf8_lossy(record.name()).to_ascii_uppercase())
        .collect()
}

/// The items of a representation, simple or complex; `None` if the
/// instance is not one.
fn items(exchange: &Exchange<'_>, instance: &Instance) -> Option<Vec<u64>> {
    let names = names(exchange, instance);
    if !names.iter().any(|n| n.ends_with("REPRESENTATION"))
        || names
            .iter()
            .any(|n| n.contains("CONTEXT") || NOT_REPRESENTATIONS.contains(&n.as_str()))
    {
        return None;
    }
    let record = if instance.is_complex() {
        exchange
            .records(instance)
            .find(|r| r.is("REPRESENTATION"))?
    } else {
        exchange.records(instance).next()?
    };
    Some(
        record
            .param(1)?
            .list()?
            .filter_map(|item| item.reference())
            .collect(),
    )
}

/// What reaches an edge first: a face, or a representation directly.
#[derive(Clone, Copy)]
enum Origin {
    Face(u64),
    Representation(u64),
}

/// Every face and edge the file's representations reach, each once, in
/// the order reached.
fn faces_and_edges(graph: &Graph<'_>) -> (Vec<u64>, Vec<(u64, Origin)>) {
    let exchange = graph.exchange();
    let mut seen = HashSet::new();
    let mut faces = Vec::new();
    let mut edges = Vec::new();
    for instance in exchange.instances() {
        let Some(items) = items(exchange, instance) else {
            continue;
        };
        let mut stack: Vec<(u64, Origin)> = items
            .into_iter()
            .rev()
            .map(|item| (item, Origin::Representation(instance.id)))
            .collect();
        while let Some((id, origin)) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(node) = exchange.get(id) else {
                continue;
            };
            let names = names(exchange, node);
            if names.iter().any(|n| {
                n.ends_with("REPRESENTATION")
                    || n.ends_with("STYLED_ITEM")
                    || n == "MAPPED_ITEM"
                    || n.ends_with("CONTEXT")
            }) {
                continue;
            }
            let simple = |name: &str| !node.is_complex() && names[0] == name;
            if simple("EDGE_CURVE") {
                edges.push((id, origin));
                continue;
            }
            let (origin, next): (Origin, Vec<u64>) =
                if simple("ADVANCED_FACE") || simple("FACE_SURFACE") {
                    faces.push(id);
                    let bounds = attribute(exchange, id, 1)
                        .and_then(|p| p.list())
                        .map(|list| list.filter_map(|p| p.reference()).collect())
                        .unwrap_or_default();
                    (Origin::Face(id), bounds)
                } else {
                    (origin, exchange.references(node).collect())
                };
            stack.extend(next.into_iter().rev().map(|id| (id, origin)));
        }
    }
    (faces, edges)
}

fn attribute<'e>(exchange: &'e Exchange<'_>, id: u64, index: usize) -> Option<Param<'e>> {
    let instance = exchange.get(id)?;
    exchange.records(instance).next()?.param(index)
}

fn reference(exchange: &Exchange<'_>, id: u64, index: usize) -> Option<u64> {
    attribute(exchange, id, index).and_then(|p| p.reference())
}

fn surface_kind(surface: &Surface) -> &'static str {
    match surface {
        Surface::Plane(_) => "plane",
        Surface::Cylindrical(_) => "cylindrical",
        Surface::Conical(_) => "conical",
        Surface::Spherical(_) => "spherical",
        Surface::Toroidal(_) => "toroidal",
        _ => "other",
    }
}

fn curve_kind(curve: &Curve) -> String {
    match curve {
        Curve::Line(_) => "line".to_owned(),
        Curve::Circle(_) => "circle".to_owned(),
        Curve::SurfaceCurve(c) => {
            let subtype = match c.subtype {
                SurfaceCurveKind::SurfaceCurve => "surface_curve",
                SurfaceCurveKind::SeamCurve => "seam_curve",
                SurfaceCurveKind::IntersectionCurve => "intersection_curve",
                _ => "surface_curve?",
            };
            format!("{subtype}({})", curve_kind(&c.curve_3d))
        }
        _ => "other".to_owned(),
    }
}

/// What a file's faces and edges read as.
#[derive(Debug, Default)]
struct Read {
    surfaces: Vec<(u64, Surface)>,
    curves: Vec<(u64, Curve)>,
    failures: Vec<String>,
}

impl Read {
    fn surface_counts(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for (_, surface) in &self.surfaces {
            *counts.entry(surface_kind(surface)).or_default() += 1;
        }
        counts
    }

    fn curve_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for (_, curve) in &self.curves {
            *counts.entry(curve_kind(curve)).or_default() += 1;
        }
        counts
    }

    fn cones(&self) -> impl Iterator<Item = f64> + '_ {
        self.surfaces.iter().filter_map(|(_, s)| match s {
            Surface::Conical(cone) => Some(cone.semi_angle),
            _ => None,
        })
    }

    fn surface_curves(&self) -> impl Iterator<Item = &SurfaceCurve> {
        self.curves.iter().filter_map(|(_, c)| match c {
            Curve::SurfaceCurve(curve) => Some(curve),
            _ => None,
        })
    }
}

/// Reads every face's surface and every edge's curve.
fn read(graph: &Graph<'_>) -> Read {
    let exchange = graph.exchange();
    let (faces, edges) = faces_and_edges(graph);
    let mut out = Read::default();
    let mut readers: HashMap<u64, Option<Reader<'_, '_>>> = HashMap::new();
    for face in faces {
        let reader = match Reader::for_item(graph, face) {
            Ok(reader) => Some(reader),
            Err(e) => {
                out.failures.push(format!("face #{face}: {e}"));
                None
            }
        };
        if let Some(reader) = &reader {
            let surface = reference(exchange, face, 2).expect("face_geometry");
            match reader.surface(surface) {
                Ok(read) => out.surfaces.push((face, read)),
                Err(e) => out.failures.push(format!("face #{face}: {e}")),
            }
        }
        readers.insert(face, reader);
    }
    let mut representations = HashMap::new();
    for (edge, origin) in edges {
        let reader = match origin {
            Origin::Face(face) => readers[&face].as_ref(),
            Origin::Representation(rep) => representations
                .entry(rep)
                .or_insert_with(|| Reader::for_representation(graph, rep).ok())
                .as_ref(),
        };
        let Some(reader) = reader else {
            out.failures.push(format!("edge #{edge}: no reader"));
            continue;
        };
        let curve = reference(exchange, edge, 3).expect("edge_geometry");
        match reader.curve(curve) {
            Ok(read) => out.curves.push((edge, read)),
            Err(e) => out.failures.push(format!("edge #{edge}: {e}")),
        }
    }
    out
}

fn read_fixture(relative: &str) -> Option<Read> {
    let src = fixture(relative)?;
    let read = read(&graph(&src));
    assert!(
        read.failures.is_empty(),
        "{relative}: {}",
        read.failures.join("; ")
    );
    Some(read)
}

const NIST: &str = "nist/NIST-PMI-STEP-Files";

#[test]
fn every_face_surface_and_edge_curve_reads() {
    let files = step_files();
    require_core_set(&files);
    let mut core: (BTreeMap<&str, usize>, BTreeMap<String, usize>) = Default::default();
    let mut all: (BTreeMap<&str, usize>, BTreeMap<String, usize>) = Default::default();
    let mut failures = Vec::new();
    for path in &files {
        let src = fs::read(path).expect("fixture is readable");
        let read = read(&graph(&src));
        if !read.failures.is_empty() {
            let shown = read.failures.len().min(5);
            failures.push(format!(
                "{}: {} failures, first: {}",
                path.display(),
                read.failures.len(),
                read.failures[..shown].join("; ")
            ));
            continue;
        }
        for totals in [&mut all]
            .into_iter()
            .chain(is_core(path).then_some(&mut core))
        {
            for (kind, n) in read.surface_counts() {
                *totals.0.entry(kind).or_default() += n;
            }
            for (kind, n) in read.curve_counts() {
                *totals.1.entry(kind).or_default() += n;
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let pin = |surfaces: &[(&'static str, usize)], curves: &[(&str, usize)]| {
        (
            surfaces.iter().copied().collect::<BTreeMap<_, _>>(),
            curves
                .iter()
                .map(|&(kind, n)| (kind.to_owned(), n))
                .collect::<BTreeMap<_, _>>(),
        )
    };
    if files.iter().filter(|path| is_core(path)).count() == common::CORE_FILES {
        assert_eq!(
            core,
            pin(
                &[
                    ("conical", 1_138),
                    ("cylindrical", 4_757),
                    ("other", 210),
                    ("plane", 3_104),
                    ("spherical", 313),
                    ("toroidal", 384),
                ],
                &[
                    ("circle", 9_476),
                    ("intersection_curve(circle)", 182),
                    ("intersection_curve(line)", 200),
                    ("line", 11_666),
                    ("other", 2_153),
                    ("surface_curve(line)", 70),
                    ("surface_curve(other)", 56),
                ],
            ),
            "face surfaces and edge curves in the core fixtures"
        );
    }
    if include_large() && files.len() == 55 {
        assert_eq!(
            all,
            pin(&ALL_SURFACES, &ALL_CURVES),
            "face surfaces and edge curves in all 55 fixtures"
        );
    }
}

/// Face surfaces per kind in all 55 fixtures: 342,733 faces, of which the
/// 10,915 stepq does not read are B-spline surfaces (5,415 simple and 1,581
/// complex rational ones), surfaces of linear extrusion (2,400) and of
/// revolution (1,519).
const ALL_SURFACES: [(&str, usize); 6] = [
    ("conical", 9_722),
    ("cylindrical", 78_152),
    ("other", 10_915),
    ("plane", 237_625),
    ("spherical", 1_229),
    ("toroidal", 5_090),
];

/// Edge curves per kind in all 55 fixtures: 979,505 edges, of which the
/// 73,525 not read are B-spline curves (72 of them complex) and 224
/// ellipses.
const ALL_CURVES: [(&str, usize); 7] = [
    ("circle", 165_819),
    ("intersection_curve(circle)", 182),
    ("intersection_curve(line)", 200),
    ("line", 739_653),
    ("other", 73_525),
    ("surface_curve(line)", 70),
    ("surface_curve(other)", 56),
];

#[test]
fn nist_ctc_04_ap242_reads_every_kind_of_elementary_surface() {
    let Some(read) = read_fixture(&format!("{NIST}/nist_ctc_04_asme1_ap242-e1.stp")) else {
        return;
    };
    assert_eq!(
        read.surface_counts(),
        BTreeMap::from([
            ("conical", 116),
            ("cylindrical", 228),
            ("plane", 89),
            ("spherical", 22),
            ("toroidal", 29),
        ])
    );
}

/// How many of `angles` (radians) are within `tolerance` rad of `degrees`.
fn at(angles: impl Iterator<Item = f64>, degrees: f64, tolerance: f64) -> usize {
    angles
        .filter(|angle| (angle - degrees * PI / 180.0).abs() <= tolerance)
        .count()
}

/// Degree factors are used as stated, and the shortest in the fixtures,
/// 0.0174532925, is 2e-11 rad off π/180 per degree: 1.2e-9 at 59 degrees.
const TEN_DIGIT_DEGREE: f64 = 2e-9;

/// A drill point is 59 degrees whether the file states 59 in a degree
/// context (NIST CTC-02 in AP242) or 1.0297 in a radian one (CTC-04).
#[test]
fn drill_points_read_as_59_degrees_in_degree_and_radian_files() {
    for (file, unit, drill_points) in [
        ("nist_ctc_02_asme1_ap242-e2.stp", "degree", 148),
        ("nist_ctc_04_asme1_ap242-e1.stp", "radian", 44),
    ] {
        let path = format!("{NIST}/{file}");
        let Some(src) = fixture(&path) else {
            continue;
        };
        let graph = graph(&src);
        let read = read(&graph);
        assert!(read.failures.is_empty(), "{file}");
        assert_eq!(at(read.cones(), 59.0, 1e-9), drill_points, "{file}");
        let (face, _) = read.surfaces[0];
        let reader = Reader::for_item(&graph, face).unwrap();
        assert_eq!(reader.units().plane_angle().unwrap().name, unit, "{file}");
    }
}

/// Small cones in degree contexts, which a guess by magnitude would read as
/// radians: NIST FTC-07's 1 and 2 degrees, the weldment's 0.57.
#[test]
fn small_degree_cones_read_as_degrees() {
    if let Some(read) = read_fixture(&format!(
        "{NIST}/AP203 geometry only/nist_ftc_07_asme1_rd.stp"
    )) {
        assert_eq!(read.cones().count(), 28);
        assert_eq!(at(read.cones(), 1.0, TEN_DIGIT_DEGREE), 4);
        assert_eq!(at(read.cones(), 2.0, TEN_DIGIT_DEGREE), 16);
        assert_eq!(at(read.cones(), 59.0, TEN_DIGIT_DEGREE), 8);
    }
    if let Some(read) = read_fixture("nist-edm/weldment_asm_solid.stp") {
        assert_eq!(at(read.cones(), 0.57, 1e-9), 8);
    }
}

/// ST-ACIS writes intersection curves whose 3D curve is the master; Matra's
/// AS1 writes surface curves whose second pcurve is.
#[test]
fn surface_curves_report_their_master_representation() {
    if let Some(read) = read_fixture("nist-edm/clevis21.stp") {
        let curves: Vec<&SurfaceCurve> = read.surface_curves().collect();
        assert_eq!(curves.len(), 78);
        assert!(curves.iter().all(|c| {
            c.subtype == SurfaceCurveKind::IntersectionCurve
                && c.master_representation == MasterRepresentation::Curve3d
        }));
        assert_eq!(
            read.curve_counts(),
            BTreeMap::from([
                ("intersection_curve(circle)".to_owned(), 30),
                ("intersection_curve(line)".to_owned(), 48),
            ])
        );
    }
    if let Some(read) = read_fixture("steptools/as1-ec-214.stp") {
        let curves: Vec<&SurfaceCurve> = read.surface_curves().collect();
        assert_eq!(curves.len(), 126);
        assert!(curves.iter().all(|c| {
            c.subtype == SurfaceCurveKind::SurfaceCurve
                && c.master_representation == MasterRepresentation::PcurveS2
        }));
        assert_eq!(
            read.curve_counts(),
            BTreeMap::from([
                ("surface_curve(line)".to_owned(), 70),
                ("surface_curve(other)".to_owned(), 56),
            ])
        );
    }
}

/// AP203 edition 1 (`CONFIG_CONTROL_DESIGN`) leaves x undefined for an
/// axis of exactly (-1, 0, 0) with no `ref_direction`; ST-ACIS writes such
/// plane positions. stepq reads the later editions' default and says that
/// x was not stated.
#[test]
fn ap203_edition_1_positions_on_minus_x_report_a_defaulted_x() {
    for (file, expected) in [("clevis21", 2), ("gear", 1), ("valve", 2)] {
        let path = format!("nist-edm/{file}.stp");
        let Some(src) = fixture(&path) else {
            continue;
        };
        let graph = graph(&src);
        let schema = graph
            .exchange()
            .header_entity("FILE_SCHEMA")
            .and_then(|r| r.param(0))
            .and_then(|p| p.list())
            .and_then(|mut list| list.next())
            .and_then(|p| p.literal())
            .map(|l| l.decode().unwrap().into_owned());
        assert_eq!(schema.as_deref(), Some("CONFIG_CONTROL_DESIGN"), "{file}");
        let read = read(&graph);
        assert!(read.failures.is_empty(), "{file}");
        let minus_x: Vec<_> = read
            .surfaces
            .iter()
            .filter_map(|(_, surface)| match surface {
                Surface::Plane(plane) => Some(&plane.position),
                _ => None,
            })
            .filter(|position| {
                position.z.map(f64::to_bits) == [-1.0f64, 0.0, 0.0].map(f64::to_bits)
                    && !position.ref_direction_stated
            })
            .collect();
        assert_eq!(minus_x.len(), expected, "{file}");
        for position in minus_x {
            assert_eq!(position.x, [0.0, 1.0, 0.0], "{file}");
            assert!(position.axis_stated);
        }
    }
}
