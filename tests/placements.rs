//! Placements and occurrences read from the fetched fixtures
//! (`stepq::model::Placements`).
//!
//! Fixtures are downloaded by `tools/fetch-fixtures.sh` and are not in git,
//! so on a fresh clone these tests find nothing and pass; with
//! `STEPQ_REQUIRE_FIXTURES=1` a missing core fixture fails them. Files over
//! 16 MB are read only with `STEPQ_LARGE_FIXTURES=1`.

mod common;

use std::collections::{BTreeMap, HashMap};
use std::fs;

use common::{fixture, include_large, is_core, require_core_set, step_files};
use stepq::geometry::Rigid;
use stepq::model::{Graph, Placement, Placements, ProductStructure};
use stepq::p21::parse;
use stepq::units::ContextUnits;

fn graph(src: &[u8]) -> Graph<'_> {
    Graph::new(parse(src).expect("fixture parses")).expect("fixture resolves")
}

/// What a set of files' usages and occurrences read as.
#[derive(Debug, Default, PartialEq)]
struct Counts {
    /// Placed through a relationship whose `rep_1` is the component's.
    normal: usize,
    /// Placed through a relationship whose `rep_1` is the assembly's.
    reversed: usize,
    /// Placed through a mapped item.
    mapped: usize,
    occurrences: usize,
}

impl Counts {
    fn add(&mut self, other: &Self) {
        self.normal += other.normal;
        self.reversed += other.reversed;
        self.mapped += other.mapped;
        self.occurrences += other.occurrences;
    }
}

/// Reads every usage and every occurrence under every root; returns the
/// counts, and every refusal.
fn read(src: &[u8]) -> (Counts, Vec<String>) {
    let graph = graph(src);
    let structure = ProductStructure::new(&graph);
    let placements = Placements::new(&graph, &structure);
    let mut counts = Counts::default();
    let mut refusals = Vec::new();
    for (index, usage) in structure.usages().iter().enumerate() {
        if let Err(error) = placements.usage(index) {
            refusals.push(error.to_string());
            continue;
        }
        match &usage.placement {
            Some(Placement::MappedItem { .. }) => counts.mapped += 1,
            Some(placement) if placement.reversed() == Some(true) => counts.reversed += 1,
            _ => counts.normal += 1,
        }
    }
    for &root in structure.roots() {
        for occurrence in placements.occurrences(root) {
            counts.occurrences += 1;
            if let Err(error) = occurrence.transform {
                refusals.push(format!("occurrence {:?}: {error}", occurrence.path));
            }
        }
    }
    (counts, refusals)
}

#[test]
fn every_usage_and_occurrence_reads() {
    let files = step_files();
    require_core_set(&files);
    let (mut core, mut all) = (Counts::default(), Counts::default());
    let mut reversed_by_file = BTreeMap::new();
    let mut failures = Vec::new();
    for path in &files {
        let src = fs::read(path).expect("fixture is readable");
        let (counts, refusals) = read(&src);
        if !refusals.is_empty() {
            failures.push(format!(
                "{}: {} refused, first: {}",
                path.display(),
                refusals.len(),
                refusals[0]
            ));
        }
        if counts.reversed > 0 {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            reversed_by_file.insert(name, counts.reversed);
        }
        all.add(&counts);
        if is_core(path) {
            core.add(&counts);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    if files.iter().filter(|path| is_core(path)).count() == common::CORE_FILES {
        assert_eq!(
            core,
            Counts {
                normal: 65,
                reversed: 131,
                mapped: 23,
                occurrences: 348,
            },
            "usages and occurrences of the core fixtures"
        );
        // Only the Pro/ENGINEER files write the assembly's representation as
        // rep_1, with its frame as transform_item_1.
        assert_eq!(
            reversed_by_file,
            BTreeMap::from([
                ("as1_pe.stp".to_owned(), 9),
                ("vaccase_asm_solid.stp".to_owned(), 14),
                ("weldment_asm_solid.stp".to_owned(), 108),
            ])
        );
    }
    if include_large() && files.len() == 55 {
        assert_eq!(
            all,
            Counts {
                normal: 3_327,
                reversed: 131,
                mapped: 23,
                occurrences: 4_359,
            },
            "usages and occurrences of all 55 fixtures"
        );
    }
}

/// Close to one of Open CASCADE's transforms (3x4, row by row) as the
/// oracle compares them: rotation entries to 1e-12, the translation to 1e-9
/// of max(1 mm, |t|).
fn close(found: &Rigid, expected: Matrix) -> bool {
    let rotation = (0..3).all(|row| {
        (0..3).all(|col| (found.rotation[row][col] - expected[row][col]).abs() <= 1e-12)
    });
    let t = expected.map(|row| row[3]);
    let size = t.iter().map(|v| v * v).sum::<f64>().sqrt();
    let moved = (0..3)
        .map(|i| (found.translation[i] - t[i]).powi(2))
        .sum::<f64>()
        .sqrt();
    rotation && moved <= 1e-9 * size.max(1.0)
}

/// Occurrences of the STEP Tools AS1 assembly by Unigraphics, as Open
/// CASCADE 8.0.1 places them: the nut-and-bolt sub-assemblies NBA1, NBA2
/// and NBA3 of both L-bracket assemblies, told apart by their NAUOs, and
/// the rod's two nuts.
#[test]
fn as1_ug_occurrences_match_open_cascade() {
    let Some(src) = fixture("steptools/as1-ug-214.stp") else {
        return;
    };
    let graph = graph(&src);
    let structure = ProductStructure::new(&graph);
    let placements = Placements::new(&graph, &structure);
    let root = structure.roots()[0];
    let by_path: HashMap<Vec<u64>, Rigid> = placements
        .occurrences(root)
        .map(|occurrence| {
            let path = occurrence
                .path
                .iter()
                .map(|&u| structure.usages()[u].instance)
                .collect();
            (path, occurrence.transform.unwrap())
        })
        .collect();
    assert_eq!(by_path.len(), 28);
    let named = |path: &[u64]| -> Vec<&str> {
        path.iter()
            .map(|id| {
                let usage = structure.usages().iter().find(|u| u.instance == *id);
                usage.and_then(|u| u.description.as_deref()).unwrap()
            })
            .collect()
    };
    for (path, names, expected) in AS1_UG {
        assert_eq!(named(path), names);
        let found = &by_path[path];
        assert!(close(found, expected), "{names:?}: {found:?}");
    }
}

/// A 3x4 transform, row by row, in mm.
type Matrix = [[f64; 4]; 3];

/// An occurrence's path of NAUO `#id`s, their descriptions, and its
/// transform to the root.
type Pinned = (&'static [u64], [&'static str; 2], Matrix);

/// Occurrences of the AS1 assembly by Unigraphics as Open CASCADE 8.0.1
/// places them: the path of NAUO `#id`s, their descriptions, and the 3x4
/// transform to the root, row by row, in mm.
const AS1_UG: [Pinned; 6] = [
    (
        &[33, 29],
        ["LB_ASSEM", "NBA3"],
        [[0., 1., 0., 25.], [1., 0., 0., 75.], [0., 0., -1., 33.]],
    ),
    (
        &[33, 30],
        ["LB_ASSEM", "NBA2"],
        [
            [0., 1., 0., 47.5],
            [1., 0., 0., 62.009_618_943_233_4],
            [0., 0., -1., 33.],
        ],
    ),
    (
        &[33, 31],
        ["LB_ASSEM", "NBA1"],
        [
            [0., 1., 0., 47.5],
            [1., 0., 0., 87.990_381_056_766_61],
            [0., 0., -1., 33.],
        ],
    ),
    (
        &[34, 31],
        ["LB_ASSEM", "NBA1"],
        [
            [0., -1., 0., 132.5],
            [-1., 0., 0., 62.009_618_943_233_4],
            [0., 0., -1., 33.],
        ],
    ),
    (
        &[35, 24],
        ["ROD_ASSEM", "NUT1"],
        [[0., 0., -1., 190.], [1., 0., 0., 65.], [0., -1., 0., 67.5]],
    ),
    (
        &[35, 25],
        ["ROD_ASSEM", "NUT2"],
        [[0., 0., -1., -7.], [1., 0., 0., 65.], [0., -1., 0., 67.5]],
    ),
];

/// Creo's Open Rack power and battery units place inch components in
/// millimetre assemblies, each frame read in its own representation's
/// units. Under 16 MB, so they are read by default when fetched.
#[test]
fn inch_components_in_millimetre_assemblies() {
    for (file, usages, mixed) in [
        (
            "ocp/ORv3_PSU_Mechanical/Mechanical/PSU, V3, 200-277V IN, 48-50V OUT, 3KW.stp",
            72,
            MIXED_PSU,
        ),
        (
            "ocp/ORv3_BBU_Mechanical/Mechanical/BATTERY BACK UP UNIT, V3 LITHIUM ION BBU, 48V, \
             3KW.stp",
            74,
            MIXED_BBU,
        ),
    ] {
        let Some(src) = fixture(file) else {
            continue;
        };
        let graph = graph(&src);
        let structure = ProductStructure::new(&graph);
        let placements = Placements::new(&graph, &structure);
        assert_eq!(structure.usages().len(), usages, "{file}");
        let factor = |representation: u64| {
            ContextUnits::of_representation(&graph, representation)
                .unwrap()
                .length()
                .unwrap()
                .factor
        };
        // The assembly side of each usage is the representation of the
        // occurrence it is a component of.
        let mut representations: HashMap<Vec<usize>, u64> = HashMap::new();
        let mut found = BTreeMap::new();
        for &root in structure.roots() {
            for occurrence in placements.occurrences(root) {
                assert!(occurrence.transform.is_ok(), "{file}: {occurrence:?}");
                let component = occurrence.representation.unwrap();
                if let Some((&usage, parent)) = occurrence.path.split_last() {
                    let assembly = representations[parent];
                    let pair = (factor(assembly).to_bits(), factor(component).to_bits());
                    if pair.0 != pair.1 {
                        found.insert(usage, (f64::from_bits(pair.0), f64::from_bits(pair.1)));
                    }
                }
                representations.insert(occurrence.path, component);
            }
        }
        let found: Vec<(f64, f64)> = found.into_values().collect();
        assert_eq!(found, mixed, "{file}");
    }
}

/// The (assembly, component) length factors of the PSU 3 kW's usages whose
/// two representations differ in length unit: four inch components in
/// millimetre assemblies.
const MIXED_PSU: &[(f64, f64)] = &[(1.0, 25.4); 4];
/// The same for the BBU 3 kW: two.
const MIXED_BBU: &[(f64, f64)] = &[(1.0, 25.4); 2];
