//! The shapes of product definitions read from the fetched fixtures
//! (`ProductStructure::shape`).
//!
//! Fixtures are downloaded by `tools/fetch-fixtures.sh` and are not in git,
//! so on a fresh clone these tests find nothing and pass; with
//! `STEPQ_REQUIRE_FIXTURES=1` a missing core fixture fails them. Files over
//! 16 MB are read only with `STEPQ_LARGE_FIXTURES=1`.

mod common;

use std::collections::{BTreeMap, HashSet};
use std::fs;

use common::{fixture, include_large, is_core, require_core_set, step_files};
use stepq::model::{DefinitionShape, Graph, ProductStructure, ShapeLink};
use stepq::p21::parse;

fn graph(src: &[u8]) -> Graph<'_> {
    Graph::new(parse(src).expect("fixture parses")).expect("fixture resolves")
}

/// The solids, as the walk counts them: instances with a partial of one of
/// these types.
const SOLIDS: &[&str] = &[
    "MANIFOLD_SOLID_BREP",
    "BREP_WITH_VOIDS",
    "FACETED_BREP",
    "FACETED_BREP_AND_BREP_WITH_VOIDS",
];

/// Every definition's shape, with its product's name.
fn shapes(src: &[u8]) -> Vec<(Option<String>, usize, DefinitionShape)> {
    let graph = graph(src);
    let structure = ProductStructure::new(&graph);
    (0..structure.definitions().len())
        .map(|d| {
            let name = structure.definitions()[d]
                .product
                .as_ref()
                .and_then(|p| p.name.clone());
            (
                name,
                structure.children(d).len(),
                structure.shape(&graph, d),
            )
        })
        .collect()
}

/// Every definition of every fixture is complete, and the definitions'
/// solids are exactly the file's solid instances, each reached from one
/// definition and one shape representation.
#[test]
fn every_solid_is_reached_once() {
    let files = step_files();
    require_core_set(&files);
    let (mut core, mut all) = ((0, 0), (0, 0));
    for path in &files {
        let src = fs::read(path).expect("fixture is readable");
        let graph = graph(&src);
        let exchange = graph.exchange();
        let structure = ProductStructure::new(&graph);
        let mut reached = Vec::new();
        for definition in 0..structure.definitions().len() {
            let shape = structure.shape(&graph, definition);
            assert!(
                shape.is_complete(),
                "{}: definition #{}: {:?}",
                path.display(),
                structure.definitions()[definition].instance,
                shape.not_followed
            );
            reached.extend(shape.solids.iter().map(|s| s.item));
        }
        let distinct: HashSet<u64> = reached.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            reached.len(),
            "{}: a solid reached twice",
            path.display()
        );
        let instances: HashSet<u64> = exchange
            .instances()
            .iter()
            .filter(|i| exchange.records(i).any(|r| SOLIDS.iter().any(|s| r.is(s))))
            .map(|i| i.id)
            .collect();
        assert_eq!(distinct, instances, "{}", path.display());
        let counts = (structure.definitions().len(), distinct.len());
        all = (all.0 + counts.0, all.1 + counts.1);
        if is_core(path) {
            core = (core.0 + counts.0, core.1 + counts.1);
        }
    }
    if files.iter().filter(|path| is_core(path)).count() == common::CORE_FILES {
        assert_eq!(core, CORE, "definitions and solids of the core fixtures");
    }
    if include_large() && files.len() == 55 {
        assert_eq!(
            all,
            (1_660, 1_261),
            "definitions and solids of all 55 fixtures"
        );
    }
}

/// Definitions and solids of the core fixtures.
const CORE: (usize, usize) = (154, 118);

/// The AS1 assembly from five exporters: 5 parts with a solid each, and 4
/// assemblies of none of their own. Unigraphics and Autodesk tie the part's
/// representation to the one holding the solid by a relationship; the
/// others list the solid directly.
#[test]
fn as1_from_five_exporters() {
    for (file, linked) in [
        ("steptools/as1-ac-214.stp", true),
        ("steptools/as1-ec-214.stp", false),
        ("steptools/as1-md-214.stp", false),
        ("steptools/as1-tc-214.stp", false),
        ("steptools/as1-ug-214.stp", true),
    ] {
        let Some(src) = fixture(file) else {
            continue;
        };
        let shapes = shapes(&src);
        let parts: Vec<&DefinitionShape> = shapes
            .iter()
            .filter(|(_, children, _)| *children == 0)
            .map(|(_, _, shape)| shape)
            .collect();
        assert_eq!(parts.len(), 5, "{file}");
        for shape in &parts {
            assert_eq!(shape.solids.len(), 1, "{file}");
            let path = &shape.solids[0].path;
            assert_eq!(path.len(), usize::from(linked), "{file}: {path:?}");
            assert!(
                path.iter()
                    .all(|link| matches!(link, ShapeLink::Relationship { .. })),
                "{file}"
            );
            assert_eq!(shape.other, []);
        }
        let assemblies = shapes.iter().filter(|(_, children, _)| *children > 0);
        for (name, _, shape) in assemblies {
            assert_eq!(shape, &DefinitionShape::default(), "{file}: {name:?}");
        }
    }
}

fn other(shape: &DefinitionShape) -> BTreeMap<&str, usize> {
    let mut found = BTreeMap::new();
    for item in &shape.other {
        *found.entry(item.entity.as_str()).or_insert(0) += 1;
    }
    found
}

/// NIST PMI files: a solid behind a relationship, next to a wireframe and
/// two surface models (and constructive planes, which are supplemental and
/// not listed); a tessellated solid, which is not a solid; and an assembly
/// of four definitions, one holding the solid.
#[test]
fn nist_pmi_shapes() {
    if let Some(src) = fixture("nist/NIST-PMI-STEP-Files/nist_ctc_02_asme1_ap242-e2.stp") {
        let shapes = shapes(&src);
        let solids: Vec<&DefinitionShape> = shapes
            .iter()
            .map(|(_, _, shape)| shape)
            .filter(|shape| !shape.solids.is_empty())
            .collect();
        assert_eq!(solids.len(), 1);
        assert_eq!(solids[0].solids.len(), 1);
        assert_eq!(solids[0].solids[0].path.len(), 1);
        assert_eq!(
            other(solids[0]),
            BTreeMap::from([("GEOMETRIC_CURVE_SET", 1), ("SHELL_BASED_SURFACE_MODEL", 2)])
        );
    }
    if let Some(src) = fixture("nist/NIST-PMI-STEP-Files/nist_ftc_08_asme1_ap242-e1-tg.stp") {
        let shapes = shapes(&src);
        assert!(shapes.iter().all(|(_, _, shape)| shape.solids.is_empty()));
        let others: usize = shapes
            .iter()
            .map(|(_, _, shape)| other(shape).get("TESSELLATED_SOLID").copied().unwrap_or(0))
            .sum();
        assert_eq!(others, 1);
    }
    if let Some(src) = fixture("nist/NIST-PMI-STEP-Files/nist_ftc_09_asme1_ap242-e1.stp") {
        let shapes = shapes(&src);
        assert_eq!(shapes.len(), 4);
        let with_solids = shapes
            .iter()
            .filter(|(_, _, shape)| !shape.solids.is_empty())
            .count();
        assert_eq!(with_solids, 1);
    }
}

/// The NIST moon buggy places its parts by mapped items: 12 parts with one
/// solid each, and 8 assemblies with none of their own.
#[test]
fn moon_buggy() {
    let Some(src) = fixture("nist-edm/moon_buggy_asm.stp") else {
        return;
    };
    let shapes = shapes(&src);
    let parts: Vec<usize> = shapes
        .iter()
        .filter(|(_, children, _)| *children == 0)
        .map(|(_, _, shape)| shape.solids.len())
        .collect();
    assert_eq!(parts, [1; 12]);
    let assemblies: Vec<&DefinitionShape> = shapes
        .iter()
        .filter(|(_, children, _)| *children > 0)
        .map(|(_, _, shape)| shape)
        .collect();
    assert_eq!(assemblies.len(), 8);
    assert!(
        assemblies
            .iter()
            .all(|shape| **shape == DefinitionShape::default())
    );
}

/// `stepq split --master` on AS1 by Unigraphics: in every master, each
/// component stub reads as defined in another file, never as empty.
#[cfg(feature = "cli")]
#[test]
fn split_masters_are_external() {
    use stepq::model::NotFollowedReason;

    let Some(src) = fixture("steptools/as1-ug-214.stp") else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("stepq-shape-master-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let input = dir.join("as1-ug.stp");
    fs::create_dir_all(&dir).unwrap();
    fs::write(&input, &src).unwrap();
    let out = dir.join("out");
    assert_cmd::Command::cargo_bin("stepq")
        .unwrap()
        .args(["split", "--master", "--out"])
        .arg(&out)
        .arg(&input)
        .assert()
        .success();
    let mut stubs = 0;
    let mut masters = 0;
    for entry in fs::read_dir(&out).unwrap() {
        let src = fs::read(entry.unwrap().path()).unwrap();
        let graph = graph(&src);
        let structure = ProductStructure::new(&graph);
        if structure.usages().is_empty() {
            continue;
        }
        masters += 1;
        for usage in structure.usages() {
            let shape = structure.shape(&graph, usage.child);
            assert_eq!(shape.solids, []);
            assert!(
                shape
                    .not_followed
                    .iter()
                    .any(|n| matches!(n.reason, NotFollowedReason::External { document: Some(_) })),
                "{shape:?}"
            );
            stubs += 1;
        }
    }
    let _ = fs::remove_dir_all(&dir);
    assert_eq!((masters, stubs), (4, 13));
}
