//! Extractions from the fetched fixtures: every output is closed and holds
//! exactly the sub-tree of the definition it was extracted for.
//!
//! This is the structural half of the split invariant; `tools/verify-split.py`
//! checks the geometric half through Open CASCADE. Fixtures come from
//! `tools/fetch-fixtures.sh`; missing files are skipped, and so are files
//! over 16 MB unless `STEPQ_LARGE_FIXTURES=1` is set.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use stepq::model::{Graph, ProductStructure, extract, orphans};
use stepq::p21::{Numbering, Writer, parse};

const FIXTURES: &[&str] = &[
    "steptools/as1-ac-214.stp",
    "steptools/as1-ec-214.stp",
    "steptools/as1-md-214.stp",
    "steptools/as1-tc-214.stp",
    "steptools/as1-ug-214.stp",
    "nist-edm/as1_pe.stp",
    "nist-edm/moon_buggy_asm.stp",
    "nist-edm/vaccase_asm_solid.stp",
    "nist-edm/weldment_asm_solid.stp",
    "nist-edm/clevis21.stp",
    "nist/NIST-PMI-STEP-Files/nist_ctc_01_asme1_ap242-e1.stp",
    "nist/NIST-PMI-STEP-Files/nist_ftc_09_asme1_ap242-e1.stp",
    // Open Rack V3 (tools/fetch-fixtures.sh ocp): real Creo assemblies with
    // up to 567 products; the four over 16 MB run only with
    // STEPQ_LARGE_FIXTURES=1. The ~200 MB Project Olympus files are left out
    // even then: extracting each of their definitions takes minutes.
    "ocp/OCP_v3_Enclosure_6OU_Section_c30001_ARP_2022.stp",
    "ocp/ORv3_BBU_Mechanical/Mechanical/BATTERY BACK UP UNIT, V3 LITHIUM ION BBU, 48V, 3KW.stp",
    "ocp/ORv3_BBU_Mechanical/Mechanical/BBU SHELF, V3, 48V, 15KW_030422.stp",
    "ocp/ORv3_PSU_Mechanical/Mechanical/POWER MODULE INTERFACE, PMI MODULE, V3.stp",
    "ocp/ORv3_PSU_Mechanical/Mechanical/POWER SHELF, V3, IEC, SINGLE INPUT, 200-277V IN 346-480V IN, 48-50V OUT, 18KW.stp",
    "ocp/ORv3_PSU_Mechanical/Mechanical/POWER SHELF, V3, NEMA, DUAL INPUT, 200-277V IN 346-480V IN, 48-50V OUT, 18KW.stp",
    "ocp/ORv3_PSU_Mechanical/Mechanical/PSU, V3, 200-277V IN, 48-50V OUT, 3KW.stp",
];

/// `id|name` of every definition in the sub-tree rooted at `root`, sorted.
fn subtree_products(structure: &ProductStructure, root: usize) -> Vec<String> {
    let mut definitions: BTreeSet<usize> = structure
        .bill_of_materials(root)
        .iter()
        .map(|line| line.definition)
        .collect();
    definitions.insert(root);
    let mut labels: Vec<String> = definitions.iter().map(|&d| label(structure, d)).collect();
    labels.sort();
    labels
}

fn label(structure: &ProductStructure, definition: usize) -> String {
    let product = structure.definitions()[definition].product.as_ref();
    format!(
        "{}|{}",
        product.and_then(|p| p.id.as_deref()).unwrap_or_default(),
        product.and_then(|p| p.name.as_deref()).unwrap_or_default()
    )
}

/// Usages whose parent is in the sub-tree rooted at `root`.
fn subtree_usages(structure: &ProductStructure, root: usize) -> usize {
    let mut inside: BTreeSet<usize> = structure
        .bill_of_materials(root)
        .iter()
        .map(|line| line.definition)
        .collect();
    inside.insert(root);
    structure
        .usages()
        .iter()
        .filter(|usage| inside.contains(&usage.parent))
        .count()
}

#[test]
fn every_extraction_is_closed_and_holds_exactly_its_subtree() {
    let root = common::fixtures_root();
    for file in FIXTURES {
        let path = root.join(file);
        let Ok(src) = fs::read(&path) else {
            eprintln!("skipping: {file} not fetched (run tools/fetch-fixtures.sh)");
            continue;
        };
        if !common::is_selected(&path) {
            eprintln!("skipping: {file} is large (set STEPQ_LARGE_FIXTURES=1)");
            continue;
        }
        let graph = Graph::new(parse(&src).unwrap()).unwrap();
        let structure = ProductStructure::new(&graph);
        let mut extractions = Vec::new();

        for (index, definition) in structure.definitions().iter().enumerate() {
            let node = graph.node(definition.instance).unwrap();
            let extraction = extract(&graph, &[node]);

            let mut out = Vec::new();
            Writer::new(graph.exchange())
                .numbering(Numbering::Dense)
                .write_pruned(
                    extraction.nodes().iter().copied(),
                    extraction.pruned().iter().copied(),
                    &mut out,
                )
                .unwrap_or_else(|e| panic!("{file} #{}: {e}", definition.instance));

            let output = Graph::new(parse(&out).unwrap()).unwrap_or_else(|e| {
                panic!("{file} #{}: output is not closed: {e}", definition.instance)
            });
            let written = ProductStructure::new(&output);
            let mut products: Vec<String> = (0..written.definitions().len())
                .map(|d| label(&written, d))
                .collect();
            products.sort();

            let expected = subtree_products(&structure, index);
            if structure.usages().is_empty() {
                // Without assembly structure, PMI may legitimately refer to
                // datums on another product definition (NIST FTC-09 does),
                // and forward references must be followed. Only require the
                // definition itself.
                assert!(
                    expected.iter().all(|label| products.contains(label)),
                    "{file} #{}: products {products:?} lack {expected:?}",
                    definition.instance
                );
            } else {
                assert_eq!(
                    products, expected,
                    "{file} #{}: products",
                    definition.instance
                );
            }
            assert_eq!(
                written.usages().len(),
                subtree_usages(&structure, index),
                "{file} #{}: usages",
                definition.instance
            );
            extractions.push(extraction);
        }

        let left = orphans(&graph, &extractions);
        let mut types: BTreeMap<String, usize> = BTreeMap::new();
        for &node in &left {
            let names: Vec<String> = graph
                .exchange()
                .records(graph.instance(node))
                .map(|r| String::from_utf8_lossy(r.name()).to_ascii_uppercase())
                .collect();
            *types.entry(names.join("+")).or_default() += 1;
        }
        eprintln!(
            "{file}: {} definitions split; {} of {} instances in no output {types:?}",
            structure.definitions().len(),
            left.len(),
            graph.len()
        );
    }
}
