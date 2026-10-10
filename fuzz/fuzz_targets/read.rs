//! Runs every reader of stated values on every instance of arbitrary bytes.
//!
//! Invariants: no reader panics, whatever the instance; what a reader
//! returns is finite (every unit factor and uncertainty is positive and
//! finite, every coordinate, length and angle finite), every direction is
//! of unit length and every placement's axes are square to each other, and
//! every error formats. Each later reader is added here.

#![no_main]

use libfuzzer_sys::fuzz_target;
use stepq::geometry::{Axis2Placement3d, Curve, GeometryError, Reader, Surface};
use stepq::model::Graph;
use stepq::p21::parse;
use stepq::units::{ContextUnits, Unit, UnitError, representation_context, unit};

/// Geometry readers built per input: enough to read in a few contexts
/// without running every reader over every instance once per instance.
const READERS: usize = 4;

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
