//! Runs every reader of stated values on every instance of arbitrary bytes.
//!
//! Invariants: no reader panics, whatever the instance; what a reader
//! returns is finite (every unit factor and uncertainty is positive and
//! finite, every coordinate, length and angle finite), every direction is
//! of unit length and every placement's axes are square to each other,
//! every usage's and occurrence's transform is finite with a rotation that
//! is square and of unit scale, each occurrence's path ends at its
//! definition, every solid's topology refers only to its own shells,
//! faces, edges and vertices, its loops close and each use of an edge is
//! by a loop that holds it, every definition's shape items start from one
//! of its shape representations by a path within the depth limit and are
//! placed by a finite transform, and every error formats. Each later
//! reader is added here.

#![no_main]

use libfuzzer_sys::fuzz_target;
use stepq::geometry::{Axis2Placement3d, Curve, GeometryError, Reader, Rigid, Surface};
use stepq::model::{Graph, PlacementError, Placements, ProductStructure, ShapeItem};
use stepq::p21::parse;
use stepq::topology::{Loop, Solid};
use stepq::units::{ContextUnits, Unit, UnitError, representation_context, unit};

/// Geometry readers built per input: enough to read in a few contexts
/// without running every reader over every instance once per instance.
const READERS: usize = 4;

/// The deepest occurrence path: 1,024 levels, and one more for a usage
/// refused as too deep (`PlacementError::TooDeep`).
const MAX_PATH: usize = 1025;

/// Occurrences read per input, over every root: a crafted assembly can
/// reach a sub-assembly by exponentially many paths, which the iterator
/// produces lazily, and the target needs only some of them.
const OCCURRENCES: usize = 4096;

fuzz_target!(|data: &[u8]| {
    let Ok(exchange) = parse(data) else {
        return;
    };
    let graph = Graph::build(exchange);
    let ids: Vec<u64> = graph.exchange().instances().iter().map(|i| i.id).collect();
    let mut readers = Vec::new();
    for &id in &ids {
        for reader in [
            Reader::for_representation(&graph, id),
            Reader::for_item(&graph, id),
        ] {
            match reader {
                Ok(reader) if readers.len() < READERS => readers.push(reader),
                Ok(_) => {}
                Err(error) => {
                    let _ = error.to_string();
                }
            }
        }
    }
    for reader in &readers {
        check_context(reader.units());
        for &id in &ids {
            check_geometry(reader, id);
        }
    }
    check_placements(&graph);
    for &id in &ids {
        match Solid::read(&graph, id) {
            Ok(solid) => check_solid(&solid),
            Err(error) => {
                let _ = error.to_string();
            }
        }
    }
    for &id in &ids {
        check_unit(unit(&graph, id));
        if let Err(error) = representation_context(&graph, id) {
            let _ = error.to_string();
        }
        for units in [
            ContextUnits::of_context(&graph, id),
            ContextUnits::of_representation(&graph, id),
            ContextUnits::of_item(&graph, id),
        ] {
            match units {
                Ok(units) => check_context(&units),
                Err(error) => {
                    let _ = error.to_string();
                }
            }
        }
    }
});

fn check_unit(result: Result<Unit, UnitError>) {
    match result {
        Ok(unit) => assert!(
            unit.factor.is_finite() && unit.factor > 0.0,
            "#{}: factor {}",
            unit.instance,
            unit.factor
        ),
        Err(error) => {
            let _ = error.to_string();
        }
    }
}

fn check_context(units: &ContextUnits) {
    for unit in [units.length(), units.plane_angle(), units.solid_angle()] {
        check_unit(unit.cloned());
    }
    match units.uncertainty() {
        Ok(uncertainties) => {
            for uncertainty in uncertainties {
                assert!(
                    uncertainty.value.is_finite() && uncertainty.value > 0.0,
                    "#{}: uncertainty {}",
                    uncertainty.instance,
                    uncertainty.value
                );
                check_unit(Ok(uncertainty.unit.clone()));
            }
        }
        Err(error) => {
            let _ = error.to_string();
        }
    }
}

fn finite(values: &[f64], what: &str) {
    assert!(values.iter().all(|v| v.is_finite()), "{what}: {values:?}");
}

fn unit_length(v: [f64; 3], what: &str) {
    finite(&v, what);
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    assert!((length - 1.0).abs() <= 1e-12, "{what}: {v:?} has length {length}");
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn check_placement(frame: &Axis2Placement3d) {
    finite(&frame.location, "location");
    for axis in [frame.x, frame.y, frame.z] {
        unit_length(axis, "placement axis");
    }
    for (a, b) in [(frame.x, frame.y), (frame.y, frame.z), (frame.z, frame.x)] {
        assert!(dot(a, b).abs() <= 1e-9, "#{}: axes not square", frame.instance);
    }
    // The inverse's translation, −Rᵀ·location, has the location's length,
    // which can exceed f64::MAX although each coordinate is finite: the
    // reader is right to return such a location. Each component sums three
    // products no larger than a coordinate, so below a quarter of f64::MAX
    // it cannot overflow.
    if frame.location.iter().all(|c| c.abs() <= f64::MAX / 4.0) {
        finite(&frame.to_rigid().inverse().translation, "inverse");
    }
}

fn check_curve(curve: &Curve) {
    match curve {
        Curve::Line(line) => {
            finite(&line.pnt, "pnt");
            unit_length(line.dir.orientation, "line direction");
            assert!(line.dir.magnitude.is_finite() && line.dir.magnitude >= 0.0);
        }
        Curve::Circle(circle) => {
            check_placement(&circle.position);
            assert!(circle.radius.is_finite() && circle.radius > 0.0);
        }
        Curve::SurfaceCurve(surface_curve) => {
            assert!(!matches!(*surface_curve.curve_3d, Curve::SurfaceCurve(_)));
            check_curve(&surface_curve.curve_3d);
        }
        _ => {}
    }
}

fn check_surface(surface: &Surface) {
    let radii: &[f64] = match surface {
        Surface::Plane(plane) => {
            check_placement(&plane.position);
            &[]
        }
        Surface::Cylindrical(s) => {
            check_placement(&s.position);
            &[s.radius]
        }
        Surface::Conical(s) => {
            check_placement(&s.position);
            assert!(s.semi_angle > 0.0 && s.semi_angle < std::f64::consts::FRAC_PI_2);
            assert!(s.radius.is_finite() && s.radius >= 0.0);
            &[]
        }
        Surface::Spherical(s) => {
            check_placement(&s.position);
            &[s.radius]
        }
        Surface::Toroidal(s) => {
            check_placement(&s.position);
            &[s.major_radius, s.minor_radius]
        }
        _ => &[],
    };
    assert!(radii.iter().all(|r| r.is_finite() && *r > 0.0), "{radii:?}");
}

fn refused(error: &GeometryError) {
    let _ = error.to_string();
}

fn check_geometry(reader: &Reader<'_, '_>, id: u64) {
    match reader.point(id) {
        Ok(point) => finite(&point, "point"),
        Err(error) => refused(&error),
    }
    match reader.direction(id) {
        Ok(direction) => unit_length(direction, "direction"),
        Err(error) => refused(&error),
    }
    match reader.vector(id) {
        Ok(vector) => {
            unit_length(vector.orientation, "vector");
            assert!(vector.magnitude.is_finite() && vector.magnitude >= 0.0);
        }
        Err(error) => refused(&error),
    }
    match reader.axis1_placement(id) {
        Ok(placement) => {
            finite(&placement.location, "location");
            unit_length(placement.axis, "axis");
        }
        Err(error) => refused(&error),
    }
    match reader.axis2_placement_3d(id) {
        Ok(placement) => check_placement(&placement),
        Err(error) => refused(&error),
    }
    match reader.surface(id) {
        Ok(surface) => check_surface(&surface),
        Err(error) => refused(&error),
    }
    match reader.curve(id) {
        Ok(curve) => check_curve(&curve),
        Err(error) => refused(&error),
    }
}

/// A transform stepq returns: finite, and a rotation that is square and of
/// unit scale. A frame whose `ref_direction` is just within the 1e-6 sine
/// policy is square only to about 1e-10, and each usage composes two
/// frames, so even 1,025 levels stay well within 1e-6.
fn check_transform(transform: &Rigid) {
    finite(&transform.translation, "translation");
    let rows = transform.rotation;
    for row in rows {
        finite(&row, "rotation");
    }
    for (i, a) in rows.iter().enumerate() {
        for (j, b) in rows.iter().enumerate() {
            let expected = if i == j { 1.0 } else { 0.0 };
            assert!(
                (dot(*a, *b) - expected).abs() <= 1e-6,
                "rotation is not orthonormal: {rows:?}"
            );
        }
    }
}

fn check_placement_result(result: &Result<Rigid, PlacementError>) {
    match result {
        Ok(transform) => check_transform(transform),
        Err(error) => {
            let _ = error.to_string();
        }
    }
}

/// The deepest shape path: 64 links.
const MAX_SHAPE_PATH: usize = 64;

/// Shape items whose transform is read per input.
const TRANSFORMS: usize = 256;

fn check_shapes(graph: &Graph<'_>, structure: &ProductStructure) {
    let mut budget = TRANSFORMS;
    for definition in 0..structure.definitions().len() {
        let shape = structure.shape(graph, definition);
        let roots = &structure.definitions()[definition].shape_representations;
        let items: Vec<&ShapeItem> = shape.solids.iter().chain(&shape.other).collect();
        for item in items {
            assert!(roots.contains(&item.shape_representation));
            assert!(item.path.len() <= MAX_SHAPE_PATH);
            assert_eq!(item.path.is_empty(), item.representation == item.shape_representation);
            if budget > 0 {
                budget -= 1;
                check_placement_result(&item.transform(graph));
            }
        }
        for not_followed in &shape.not_followed {
            let _ = format!("{not_followed:?}");
        }
    }
}

fn check_placements(graph: &Graph<'_>) {
    let structure = ProductStructure::new(graph);
    check_shapes(graph, &structure);
    let placements = Placements::new(graph, &structure);
    for usage in 0..structure.usages().len() {
        check_placement_result(&placements.usage(usage));
    }
    let mut budget = OCCURRENCES;
    // Every root, and the first definition, which in a file where every
    // definition is a component (a cycle) is no root.
    let starts = structure
        .roots()
        .iter()
        .copied()
        .chain((!structure.definitions().is_empty()).then_some(0));
    for root in starts {
        for occurrence in placements.occurrences(root).take(budget) {
            budget -= 1;
            check_placement_result(&occurrence.transform);
            assert!(occurrence.path.len() <= MAX_PATH);
            match occurrence.path.last() {
                Some(&usage) => assert_eq!(structure.usages()[usage].child, occurrence.definition),
                None => assert_eq!(occurrence.definition, root),
            }
        }
    }
}

fn check_solid(solid: &Solid) {
    let id = solid.instance;
    let (faces, edges, vertices) = (
        solid.faces().len(),
        solid.edges().len(),
        solid.vertices().len(),
    );
    assert!(!solid.shells().is_empty());
    // The shells' faces partition the faces, in order.
    let mut next = 0;
    for (index, shell) in solid.shells().iter().enumerate() {
        assert_eq!(shell.faces.start, next, "#{id}");
        assert!(!shell.faces.is_empty(), "#{id}: an empty shell");
        next = shell.faces.end;
        for face in shell.faces.clone() {
            assert_eq!(solid.faces()[face].shell, index, "#{id}");
        }
    }
    assert_eq!(next, faces, "#{id}");
    for edge in solid.edges() {
        assert!(edge.start < vertices && edge.end < vertices, "#{id}");
    }
    for (index, face) in solid.faces().iter().enumerate() {
        assert!(!face.bounds.is_empty(), "#{id}");
        assert!(face.bounds.iter().filter(|b| b.outer).count() <= 1, "#{id}");
        for bound in &face.bounds {
            match &bound.bound {
                Loop::EdgeLoop { edges: list, .. } => {
                    assert!(!list.is_empty(), "#{id}");
                    for (k, oriented) in list.iter().enumerate() {
                        assert!(oriented.edge < edges, "#{id}");
                        let next = &list[(k + 1) % list.len()];
                        assert_eq!(oriented.end, next.start, "#{id}: a loop that does not close");
                    }
                }
                Loop::VertexLoop { vertex, .. } => assert!(*vertex < vertices, "#{id}"),
                _ => {}
            }
        }
        let _ = solid.face_sense(index);
        assert!(!solid.neighbours(index).contains(&index), "#{id}");
        assert!(solid.face_vertices(index).iter().all(|&v| v < vertices), "#{id}");
    }
    // Each use names a loop that holds the edge there.
    let mut uses = 0;
    for edge in 0..edges {
        for used in solid.uses(edge) {
            let bound = &solid.faces()[used.face].bounds[used.bound];
            let Loop::EdgeLoop { edges: list, .. } = &bound.bound else {
                panic!("#{id}: a use of an edge by a vertex loop");
            };
            assert_eq!(list[used.index].edge, edge, "#{id}");
            uses += 1;
        }
        let _ = solid.is_seam(edge);
    }
    let held: usize = solid
        .faces()
        .iter()
        .flat_map(|face| &face.bounds)
        .map(|bound| match &bound.bound {
            Loop::EdgeLoop { edges: list, .. } => list.len(),
            _ => 0,
        })
        .sum();
    assert_eq!(uses, held, "#{id}: every oriented edge is one use");
}
