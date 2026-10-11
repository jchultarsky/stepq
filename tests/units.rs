//! Unit factors read from the fetched fixtures (`stepq::units`).
//!
//! Fixtures are downloaded by `tools/fetch-fixtures.sh` and are not in git,
//! so on a fresh clone these tests find nothing and pass; with
//! `STEPQ_REQUIRE_FIXTURES=1` a missing core fixture fails them. Files over
//! 16 MB are read only with `STEPQ_LARGE_FIXTURES=1`. Factors are compared
//! bit for bit: a factor is the stated value times its base unit's factor,
//! computed here the same way.

mod common;

use std::collections::BTreeMap;
use std::fs;

use common::{fixture, include_large, is_core, require_core_set, step_files};
use stepq::model::Graph;
use stepq::p21::{Record, parse};
use stepq::units::{ContextUnits, UnitError, UnitKind, representation_context, unit};

fn graph(src: &[u8]) -> Graph<'_> {
    Graph::new(parse(src).expect("fixture parses")).expect("fixture resolves")
}

/// The contexts that assign units, in file order.
fn unit_contexts(graph: &Graph<'_>) -> Vec<ContextUnits> {
    let exchange = graph.exchange();
    exchange
        .instances_of("GLOBAL_UNIT_ASSIGNED_CONTEXT")
        .map(|node| {
            let id = graph.instance(node).id;
            ContextUnits::of_context(graph, id).unwrap_or_else(|e| panic!("#{id}: {e}"))
        })
        .collect()
}

fn factor_bits(result: Result<&stepq::units::Unit, UnitError>) -> u64 {
    result.expect("unit reads").factor.to_bits()
}

#[test]
fn every_context_reads_its_length_and_plane_angle_units() {
    let files = step_files();
    require_core_set(&files);
    let mut core = 0;
    let mut all = 0;
    let mut failures = Vec::new();
    for path in &files {
        let src = fs::read(path).expect("fixture is readable");
        let graph = graph(&src);
        for units in unit_contexts(&graph) {
            all += 1;
            if is_core(path) {
                core += 1;
            }
            let solid_angle = match units.solid_angle() {
                Ok(_) | Err(UnitError::Missing { .. }) => Ok(()),
                Err(e) => Err(e),
            };
            let problems: Vec<String> = [
                units.length().map(|_| ()),
                units.plane_angle().map(|_| ()),
                solid_angle,
                units.uncertainty().map(|_| ()),
            ]
            .into_iter()
            .filter_map(Result::err)
            .map(|e| e.to_string())
            .collect();
            if !problems.is_empty() {
                failures.push(format!("{}: {}", path.display(), problems.join("; ")));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    if files.iter().filter(|path| is_core(path)).count() == common::CORE_FILES {
        assert_eq!(core, 159, "unit contexts in the core fixtures");
    }
    if include_large() && files.len() == 55 {
        assert_eq!(all, 1_670, "unit contexts in all 55 fixtures");
    }
}

/// Pro/ENGINEER's AS1: every context in inches with a 13-digit degree, and
/// a 'closure' uncertainty that differs per context, each stated in inches.
#[test]
fn as1_pe_states_inches_and_a_closure_per_context() {
    let Some(src) = fixture("nist-edm/as1_pe.stp") else {
        return;
    };
    let graph = graph(&src);
    let contexts = unit_contexts(&graph);
    assert_eq!(contexts.len(), 7);
    let mut closures = BTreeMap::new();
    for units in &contexts {
        let inch = units.length().unwrap();
        assert_eq!((inch.kind, inch.name.as_str()), (UnitKind::Length, "inch"));
        assert_eq!(inch.factor.to_bits(), 25.4f64.to_bits());
        assert_eq!(
            factor_bits(units.plane_angle()),
            1.745_329_251_994e-2f64.to_bits()
        );
        assert_eq!(units.solid_angle().unwrap().name, "steradian");
        let [closure] = units.uncertainty().unwrap() else {
            panic!("#{}: one uncertainty", units.context);
        };
        assert_eq!(closure.name, "closure");
        *closures.entry(closure.value.to_bits()).or_insert(0) += 1;
    }
    let expected = BTreeMap::from([
        ((2.646_949_085_119e-2 * 25.4f64).to_bits(), 3),
        ((1.427_458_223_298e-2 * 25.4f64).to_bits(), 3),
        ((4.527_511_461_366e-3 * 25.4f64).to_bits(), 1),
    ]);
    assert_eq!(closures, expected, "0.672, 0.363 and 0.115 mm");
}

/// `as1-ac-214` writes the degree as 0.0174532925 radian; it is used as
/// written, never snapped to π/180.
#[test]
fn as1_ac_degree_is_used_as_written() {
    let Some(src) = fixture("steptools/as1-ac-214.stp") else {
        return;
    };
    let graph = graph(&src);
    let contexts = unit_contexts(&graph);
    assert_eq!(contexts.len(), 9);
    for units in contexts {
        let degree = units.plane_angle().unwrap();
        assert_eq!(degree.name, "degree");
        assert_eq!(degree.factor.to_bits(), 0.017_453_292_5f64.to_bits());
    }
}

/// NIST FTC-07 in AP203: an inch defined as 2.54 centimetres, next to a
/// millimetre context.
#[test]
fn nist_ftc_07_defines_the_inch_through_the_centimetre() {
    let Some(src) =
        fixture("nist/NIST-PMI-STEP-Files/AP203 geometry only/nist_ftc_07_asme1_rd.stp")
    else {
        return;
    };
    let graph = graph(&src);
    let inch = ContextUnits::of_context(&graph, 42).unwrap();
    let length = inch.length().unwrap();
    assert_eq!(length.name, "inch");
    assert_eq!(length.factor.to_bits(), (2.54 * 10.0f64).to_bits());
    let centimetre = unit(&graph, 32).unwrap();
    assert_eq!(
        (centimetre.name.as_str(), centimetre.factor),
        ("centimetre", 10.0)
    );
    let [tolerance] = inch.uncertainty().unwrap() else {
        panic!("one uncertainty");
    };
    assert_eq!(
        tolerance.value.to_bits(),
        (0.000_393_700_787_402 * (2.54 * 10.0f64)).to_bits()
    );
    let millimetre = ContextUnits::of_context(&graph, 73).unwrap();
    assert_eq!(
        millimetre.length().unwrap().factor.to_bits(),
        1.0f64.to_bits()
    );
    assert_eq!(
        factor_bits(millimetre.plane_angle()),
        0.017_453_292_5f64.to_bits()
    );
}

/// NIST CTC-05 in AP242 defines its inch through a plain
/// `MEASURE_WITH_UNIT`, which states no kind; the unit's own record does.
#[test]
fn nist_ctc_05_defines_the_inch_through_a_plain_measure_with_unit() {
    let Some(src) = fixture("nist/NIST-PMI-STEP-Files/nist_ctc_05_asme1_ap242-e1.stp") else {
        return;
    };
    let graph = graph(&src);
    let inches: Vec<ContextUnits> = unit_contexts(&graph)
        .into_iter()
        .filter(|units| units.length().is_ok_and(|unit| unit.name == "inch"))
        .collect();
    assert!(!inches.is_empty(), "an inch context");
    for units in inches {
        assert_eq!(factor_bits(units.length()), 25.4f64.to_bits());
    }
}

/// Matra's AS1 writes 252 parametric 2D contexts for its pcurves. They
/// assign no units, and every kind reads as missing rather than as a
/// default.
#[test]
fn as1_ec_parametric_contexts_assign_no_units() {
    let Some(src) = fixture("steptools/as1-ec-214.stp") else {
        return;
    };
    let graph = graph(&src);
    let exchange = graph.exchange();
    let parametric: Vec<u64> = exchange
        .instances_of("PARAMETRIC_REPRESENTATION_CONTEXT")
        .map(|node| graph.instance(node).id)
        .collect();
    assert_eq!(parametric.len(), 252);
    for id in parametric {
        let units = ContextUnits::of_context(&graph, id).unwrap();
        assert_eq!(units.dimension, Some(2));
        for kind in [UnitKind::Length, UnitKind::PlaneAngle, UnitKind::SolidAngle] {
            assert_eq!(
                units.unit(kind),
                Err(UnitError::Missing { context: id, kind }),
                "#{id}"
            );
        }
        assert_eq!(units.uncertainty(), Ok(&[][..]));
    }
}

/// CADDS via Theorem states no uncertainty in nine of its contexts: they
/// read as an empty list, not as an error.
#[test]
fn as1_tc_states_no_uncertainty_in_nine_contexts() {
    let Some(src) = fixture("steptools/as1-tc-214.stp") else {
        return;
    };
    let graph = graph(&src);
    let empty = unit_contexts(&graph)
        .iter()
        .filter(|units| units.uncertainty().unwrap().is_empty())
        .count();
    assert_eq!(empty, 9);
}

/// What the item walk counts in one file.
#[derive(Default)]
struct Items {
    representations: usize,
    solids: usize,
    faces: usize,
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

fn names(records: &[Record<'_>]) -> Vec<String> {
    records
        .iter()
        .map(|r| String::from_utf8_lossy(r.name()).to_ascii_uppercase())
        .collect()
}

/// Resolves the context of every representation and the units of every
/// solid, surface model and face; returns the counts or the failures.
fn walk_items(graph: &Graph<'_>) -> Result<Items, Vec<String>> {
    let exchange = graph.exchange();
    let mut items = Items::default();
    let mut failures = Vec::new();
    for instance in exchange.instances() {
        let records: Vec<Record<'_>> = exchange.records(instance).collect();
        let names = names(&records);
        let is_representation = names.iter().any(|n| n.ends_with("REPRESENTATION"))
            && !names
                .iter()
                .any(|n| n.contains("CONTEXT") || NOT_REPRESENTATIONS.contains(&n.as_str()));
        if is_representation {
            items.representations += 1;
            if let Err(e) = representation_context(graph, instance.id) {
                failures.push(format!("representation #{}: {e}", instance.id));
            }
        }
        let solid = names.iter().any(|n| {
            matches!(
                n.as_str(),
                "MANIFOLD_SOLID_BREP"
                    | "BREP_WITH_VOIDS"
                    | "FACETED_BREP"
                    | "SHELL_BASED_SURFACE_MODEL"
            )
        });
        let face = names
            .iter()
            .any(|n| matches!(n.as_str(), "ADVANCED_FACE" | "FACE_SURFACE"));
        if solid || face {
            if solid {
                items.solids += 1;
            } else {
                items.faces += 1;
            }
            match ContextUnits::of_item(graph, instance.id) {
                Ok(units) => {
                    if let Err(e) = units.length() {
                        failures.push(format!("#{}: {e}", instance.id));
                    }
                }
                Err(e) => failures.push(format!("#{}: {e}", instance.id)),
            }
        }
    }
    if failures.is_empty() {
        Ok(items)
    } else {
        Err(failures)
    }
}

#[test]
fn every_representation_solid_and_face_resolves_its_units() {
    let files = step_files();
    require_core_set(&files);
    let mut core = Items::default();
    let mut all = Items::default();
    let mut failures = Vec::new();
    for path in &files {
        let src = fs::read(path).expect("fixture is readable");
        match walk_items(&graph(&src)) {
            Ok(items) => {
                for total in [&mut all]
                    .into_iter()
                    .chain(is_core(path).then_some(&mut core))
                {
                    total.representations += items.representations;
                    total.solids += items.solids;
                    total.faces += items.faces;
                }
            }
            Err(problems) => {
                let shown = problems.len().min(5);
                failures.push(format!(
                    "{}: {} items, first: {}",
                    path.display(),
                    problems.len(),
                    problems[..shown].join("; ")
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    if files.iter().filter(|path| is_core(path)).count() == common::CORE_FILES {
        assert_eq!(
            (core.representations, core.solids, core.faces),
            CORE_ITEMS,
            "representations, solids and faces in the core fixtures"
        );
    }
    if include_large() && files.len() == 55 {
        assert_eq!(
            (all.representations, all.solids, all.faces),
            (24_362, 1_348, 342_733),
            "representations, solids and faces in all 55 fixtures"
        );
    }
}

const CORE_ITEMS: (usize, usize, usize) = (3_253, 149, 9_906);

/// `rep_1` and `rep_2` of every representation relationship with
/// transformation, simple or complex.
fn transformed_pairs(graph: &Graph<'_>) -> Vec<(u64, u64, u64)> {
    let exchange = graph.exchange();
    let mut pairs = Vec::new();
    for instance in exchange.instances() {
        let records: Vec<Record<'_>> = exchange.records(instance).collect();
        let base = if instance.is_complex() {
            records
                .iter()
                .any(|r| r.is("REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION"))
                .then(|| records.iter().find(|r| r.is("REPRESENTATION_RELATIONSHIP")))
                .flatten()
        } else {
            records
                .first()
                .filter(|r| r.is("REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION"))
        };
        let Some(base) = base else {
            continue;
        };
        let rep = |index| base.param(index).and_then(|p| p.reference()).unwrap();
        pairs.push((instance.id, rep(2), rep(3)));
    }
    pairs
}

/// Open Rack PSU and BBU (3 kW) place inch components in millimetre
/// assemblies: the two representations of such a placement are in contexts
/// with different length units, which every placement must read in its own.
#[test]
fn open_rack_3kw_places_inch_parts_in_millimetre_assemblies() {
    let cases = [
        (
            "ocp/ORv3_PSU_Mechanical/Mechanical/PSU, V3, 200-277V IN, 48-50V OUT, 3KW.stp",
            72,
            4,
        ),
        (
            "ocp/ORv3_BBU_Mechanical/Mechanical/BATTERY BACK UP UNIT, V3 LITHIUM ION BBU, 48V, 3KW.stp",
            74,
            2,
        ),
    ];
    for (file, placements, mixed) in cases {
        let Some(src) = fixture(file) else {
            continue;
        };
        let graph = graph(&src);
        let pairs = transformed_pairs(&graph);
        assert_eq!(pairs.len(), placements, "{file}");
        let mut differing = Vec::new();
        for (relationship, rep_1, rep_2) in pairs {
            let length = |rep| {
                ContextUnits::of_representation(&graph, rep)
                    .and_then(|units| units.length().map(|unit| unit.factor.to_bits()))
                    .unwrap_or_else(|e| panic!("#{relationship}: #{rep}: {e}"))
            };
            let (a, b) = (length(rep_1), length(rep_2));
            if a != b {
                let mut factors = [f64::from_bits(a), f64::from_bits(b)];
                factors.sort_by(f64::total_cmp);
                differing.push(factors.map(f64::to_bits));
            }
        }
        assert_eq!(differing.len(), mixed, "{file}");
        assert!(
            differing
                .iter()
                .all(|pair| *pair == [1.0f64.to_bits(), 25.4f64.to_bits()]),
            "{file}: inch against millimetre"
        );
    }
}

/// The Open Rack enclosure (40 MB) has foot contexts, the only ones in the
/// fixtures, and inch contexts beside its millimetre ones.
#[test]
fn open_rack_enclosure_has_foot_and_inch_contexts() {
    let Some(src) = fixture("ocp/OCP_v3_Enclosure_6OU_Section_c30001_ARP_2022.stp") else {
        return;
    };
    let graph = graph(&src);
    let mut by_name: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    for units in unit_contexts(&graph) {
        let length = units.length().unwrap();
        let entry = by_name
            .entry(length.name.clone())
            .or_insert((0, length.factor.to_bits()));
        entry.0 += 1;
        assert_eq!(entry.1, length.factor.to_bits(), "{}", length.name);
    }
    assert_eq!(by_name["foot"], (20, (3.048e2f64).to_bits()));
    assert_eq!(by_name["inch"], (5, 25.4f64.to_bits()));
}
