//! Product structure against the fetched assembly fixtures.
//!
//! Fixtures are downloaded by `tools/fetch-fixtures.sh` and are not in git;
//! each test skips files that are missing.

use std::fs;
use std::path::Path;

use stepq::info::Info;
use stepq::model::{Graph, ProductStructure};
use stepq::p21::parse;

fn structure_of(relative: &str) -> Option<(ProductStructure, Info)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative);
    let Ok(src) = fs::read(&path) else {
        eprintln!(
            "skipping: {} not fetched (run tools/fetch-fixtures.sh)",
            path.display()
        );
        return None;
    };
    let graph = Graph::new(parse(&src).unwrap()).unwrap();
    Some((ProductStructure::new(&graph), Info::new(&graph)))
}

/// The STEP Tools AS1 assembly, as written by five exporters: a plate, two
/// L-bracket sub-assemblies each holding three nut-bolt sub-assemblies, and
/// a rod sub-assembly with two nuts. Every exporter must yield the same
/// structure and quantities.
#[test]
fn as1_has_the_same_structure_from_every_exporter() {
    for file in [
        "steptools/as1-ac-214.stp",
        "steptools/as1-ec-214.stp",
        "steptools/as1-md-214.stp",
        "steptools/as1-tc-214.stp",
        "steptools/as1-ug-214.stp",
    ] {
        let Some((structure, _)) = structure_of(file) else {
            continue;
        };
        assert_eq!(structure.usages().len(), 13, "{file}");
        let roots: Vec<usize> = structure
            .roots()
            .iter()
            .copied()
            .filter(|&root| structure.children(root).len() > 0)
            .collect();
        assert_eq!(roots.len(), 1, "{file}: one top-level assembly");

        let mut quantities: Vec<f64> = structure
            .bill_of_materials(roots[0])
            .iter()
            .map(|line| line.quantity)
            .collect();
        quantities.sort_by(f64::total_cmp);
        let quantities: Vec<String> = quantities.iter().map(ToString::to_string).collect();
        assert_eq!(
            quantities,
            ["1", "1", "1", "2", "2", "6", "6", "8"],
            "{file}"
        );

        assert!(
            structure.usages().iter().all(|u| u.placement.is_some()),
            "{file}: every usage is placed"
        );
    }
}

/// Every assembly fixture yields one usage per NAUO counted by `info`, with
/// each usage's parent and child distinct, and a bill of materials for each
/// top-level assembly.
#[test]
fn assembly_fixtures_are_consistent_with_info() {
    for file in [
        "steptools/as1-ac-214.stp",
        "steptools/as1-ec-214.stp",
        "steptools/as1-md-214.stp",
        "steptools/as1-tc-214.stp",
        "steptools/as1-ug-214.stp",
        "nist-edm/as1_pe.stp",
        "nist-edm/moon_buggy_asm.stp",
        "nist-edm/vaccase_asm_solid.stp",
        "nist-edm/weldment_asm_solid.stp",
    ] {
        let Some((structure, info)) = structure_of(file) else {
            continue;
        };
        assert_eq!(structure.usages().len(), info.assembly_usages, "{file}");
        assert!(
            structure.usages().iter().all(|u| u.parent != u.child),
            "{file}"
        );
        let reversed = structure
            .usages()
            .iter()
            .filter(|u| u.placement.as_ref().and_then(|p| p.reversed) == Some(true))
            .count();
        let placed = structure
            .usages()
            .iter()
            .filter(|u| u.placement.is_some())
            .count();
        let components: usize = structure
            .roots()
            .iter()
            .map(|&root| structure.bill_of_materials(root).len())
            .sum();
        eprintln!(
            "{file}: {} definitions, {} usages ({placed} placed, {reversed} reversed), {} top-level, {components} BOM lines",
            structure.definitions().len(),
            structure.usages().len(),
            structure.roots().len(),
        );
        assert!(components > 0, "{file}");
    }
}
