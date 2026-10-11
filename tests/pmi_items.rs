//! The representation items PMI stands for, read from the fetched fixtures
//! (`stepq::pmi::AspectItems`).
//!
//! Fixtures are downloaded by `tools/fetch-fixtures.sh` and are not in git,
//! so on a fresh clone these tests find nothing and pass; with
//! `STEPQ_REQUIRE_FIXTURES=1` a missing core fixture fails them. Files over
//! 16 MB are read only with `STEPQ_LARGE_FIXTURES=1`.

mod common;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;

use common::{fixture, require_core_set, step_files};
use stepq::model::{Graph, ProductStructure};
use stepq::p21::parse;
use stepq::pmi::{
    AspectItems, ItemUsageKind, Resolution, StepKind, StepKinds, UnresolvedReason, pmi,
};

const NIST: &str = "nist/NIST-PMI-STEP-Files/";

/// The 17 NIST AP242 files.
const AP242: &[&str] = &[
    "nist_ctc_01_asme1_ap242-e1.stp",
    "nist_ctc_02_asme1_ap242-e2.stp",
    "nist_ctc_03_asme1_ap242-e2.stp",
    "nist_ctc_04_asme1_ap242-e1.stp",
    "nist_ctc_05_asme1_ap242-e1.stp",
    "nist_ftc_06_asme1_ap242-e2.stp",
    "nist_ftc_07_asme1_ap242-e2.stp",
    "nist_ftc_08_asme1_ap242-e1-tg.stp",
    "nist_ftc_08_asme1_ap242-e2.stp",
    "nist_ftc_09_asme1_ap242-e1.stp",
    "nist_ftc_10_asme1_ap242-e2.stp",
    "nist_ftc_11_asme1_ap242-e2.stp",
    "nist_stc_06_asme1_ap242-e3.stp",
    "nist_stc_07_asme1_ap242-e3.stp",
    "nist_stc_08_asme1_ap242-e3.stp",
    "nist_stc_09_asme1_ap242-e3.stp",
    "nist_stc_10_asme1_ap242-e2.stp",
];

/// What a definition reaches: `face` if any face, `other` if only edges,
/// curves or points, else why its routes stopped.
fn class(resolution: &Resolution) -> String {
    if resolution
        .items
        .iter()
        .any(|item| item.entity.contains("FACE"))
    {
        return "face".to_owned();
    }
    if !resolution.items.is_empty() {
        return "other".to_owned();
    }
    let reasons: BTreeSet<&str> = resolution
        .unresolved
        .iter()
        .map(|unresolved| match unresolved.reason {
            UnresolvedReason::NoUsage => "no usage",
            UnresolvedReason::WholeShape => "whole shape",
            UnresolvedReason::NotFollowed { .. } => "not followed",
            _ => "other reason",
        })
        .collect();
    reasons.into_iter().collect::<Vec<_>>().join(", ")
}

/// Every tolerance target and dimension end of the 17 files, and every
/// datum, by what it reaches; every target and datum has a product
/// definition.
#[test]
fn nist_pmi_reaches_its_faces() {
    let mut ends: BTreeMap<String, usize> = BTreeMap::new();
    let mut datums: BTreeMap<String, usize> = BTreeMap::new();
    let mut read = 0;
    for file in AP242 {
        let Some(src) = fixture(&format!("{NIST}{file}")) else {
            continue;
        };
        read += 1;
        let graph = Graph::new(parse(&src).unwrap()).unwrap();
        let structure = ProductStructure::new(&graph);
        let found = pmi(&graph, &structure);
        let index = AspectItems::new(&graph);
        let count = |counts: &mut BTreeMap<String, usize>, id: u64| {
            *counts.entry(class(&index.resolve(&graph, id))).or_insert(0) += 1;
        };
        for tolerance in &found.tolerances {
            assert!(
                tolerance.target.product_definition.is_some(),
                "{file}: {tolerance:?}"
            );
            count(&mut ends, tolerance.target.instance);
        }
        for dimension in &found.dimensions {
            assert!(
                dimension.target.product_definition.is_some(),
                "{file}: {dimension:?}"
            );
            count(&mut ends, dimension.target.instance);
            if let Some(related) = &dimension.related {
                assert!(
                    related.product_definition.is_some(),
                    "{file}: {dimension:?}"
                );
                count(&mut ends, related.instance);
            }
        }
        for datum in &found.datums {
            assert!(
                datum.subject.product_definition.is_some(),
                "{file}: {datum:?}"
            );
            count(&mut datums, datum.instance);
        }
    }
    if read < AP242.len() {
        return;
    }
    let pinned = |pairs: &[(&str, usize)]| -> BTreeMap<String, usize> {
        pairs.iter().map(|&(k, v)| (k.to_owned(), v)).collect()
    };
    // 362 tolerance targets and 321 dimension ends.
    assert_eq!(
        ends,
        pinned(&[
            ("face", 637),
            ("other", 42),
            ("whole shape", 2),
            ("no usage", 2)
        ])
    );
    // 22 stop at the 'datum feature' convention of ftc 08, ftc 11 and stc
    // 10; 2 at datum targets with no feature, in ctc 05.
    assert_eq!(
        datums,
        pinned(&[
            ("face", 83),
            ("other", 7),
            ("not followed", 22),
            ("no usage", 2)
        ])
    );
}

fn read(file: &str) -> Option<(Graph<'static>, AspectItems)> {
    let src: &'static [u8] = Box::leak(fixture(&format!("{NIST}{file}"))?.into_boxed_slice());
    let graph = Graph::new(parse(src).unwrap()).unwrap();
    let index = AspectItems::new(&graph);
    Some((graph, index))
}

fn items(resolution: &Resolution, entity: &str) -> BTreeSet<u64> {
    resolution
        .items
        .iter()
        .filter(|item| item.entity == entity)
        .map(|item| item.item)
        .collect()
}

#[test]
fn nist_spot_checks() {
    if let Some((graph, index)) = read("nist_stc_10_asme1_ap242-e2.stp") {
        // A 'Radial Hole and Thread Callout' group: two cylinders, two cones
        // and four curves, each a member of the group.
        let callout = index.resolve(&graph, 814);
        assert_eq!(
            items(&callout, "ADVANCED_FACE"),
            BTreeSet::from([7039, 7040, 7186, 7187])
        );
        assert_eq!(
            items(&callout, "TRIMMED_CURVE"),
            BTreeSet::from([2218, 2219, 2220, 2221])
        );
        assert!(
            callout
                .items
                .iter()
                .all(|item| { item.route_kinds == [StepKinds::NONE.with(StepKind::Component)] })
        );
        assert_eq!(
            callout.item_ids(StepKinds::NONE),
            Vec::<u64>::new(),
            "none direct"
        );
        // Datum feature #121 points at the toleranced aspect with a plain
        // relationship described only as 'datum feature'.
        let datum = index.resolve(&graph, 499);
        assert_eq!(datum.items, []);
        let stop = &datum.unresolved[0];
        assert_eq!(stop.aspect, 121);
        assert_eq!(
            stop.reason,
            UnresolvedReason::NotFollowed {
                relationship: 759,
                related: 2923
            }
        );
    }
    if let Some((graph, index)) = read("nist_ftc_07_asme1_ap242-e2.stp") {
        let whole = index.resolve(&graph, 15);
        assert_eq!(whole.unresolved[0].reason, UnresolvedReason::WholeShape);
    }
    if let Some((graph, index)) = read("nist_ftc_06_asme1_ap242-e2.stp") {
        let bare = index.resolve(&graph, 2818);
        assert_eq!(bare.unresolved[0].reason, UnresolvedReason::NoUsage);
        // A 'semantic text' property '#{counterbore}' on four cylinders.
        let counterbore = index.resolve(&graph, 982);
        assert_eq!(
            items(&counterbore, "ADVANCED_FACE"),
            BTreeSet::from([3218, 3219, 3229, 3230])
        );
    }
    if let Some((graph, index)) = read("nist_stc_07_asme1_ap242-e3.stp") {
        // A group of two aspects, each naming its faces by an
        // item_identified_representation_usage with a set.
        let set = index.resolve(&graph, 14105);
        assert_eq!(
            items(&set, "ADVANCED_FACE"),
            BTreeSet::from([6682, 6706, 6792, 6816])
        );
        for item in &set.items {
            let usage = index
                .usages()
                .iter()
                .find(|u| u.instance == item.usage)
                .unwrap();
            assert_eq!(usage.kind, ItemUsageKind::ItemIdentified);
            assert!(usage.items.len() > 1);
        }
        // A size with a datum feature, which applies to itself.
        let size = index.resolve(&graph, 14028);
        assert_eq!(items(&size, "ADVANCED_FACE"), BTreeSet::from([6737, 6761]));
        // A directed location, from #8812 to #6908.
        let structure = ProductStructure::new(&graph);
        let found = pmi(&graph, &structure);
        // The directed locations and sizes with datum features that pmi()
        // reads besides the 39 dimensions it read before.
        let dimensions: BTreeSet<u64> = found.dimensions.iter().map(|d| d.instance).collect();
        assert_eq!(dimensions.len(), 45);
        for id in [14_028, 14_030, 16_317, 16_368, 16_419, 16_470] {
            assert!(dimensions.contains(&id), "#{id}");
        }
        let directed = found
            .dimensions
            .iter()
            .find(|d| d.instance == 16_317)
            .unwrap();
        let from = index.resolve(&graph, directed.target.instance);
        let to = index.resolve(&graph, directed.related.as_ref().unwrap().instance);
        assert_eq!(items(&from, "ADVANCED_FACE"), BTreeSet::from([8812]));
        assert_eq!(items(&to, "ADVANCED_FACE"), BTreeSet::from([6908]));
    }
}

/// On every fixture: no panic, every item is defined, and `naming` is the
/// inverse of `resolve`, with the same kinds of route: every item a datum,
/// tolerance target, dimension end or usage's aspect lists names it back,
/// and every definition `naming` lists for an item lists it.
#[test]
fn naming_inverts_resolve_on_every_fixture() {
    let files = step_files();
    require_core_set(&files);
    for path in &files {
        let src = fs::read(path).unwrap();
        let graph = Graph::new(parse(&src).unwrap()).unwrap();
        let index = AspectItems::new(&graph);
        let structure = ProductStructure::new(&graph);
        let found = pmi(&graph, &structure);
        let mut definitions: BTreeSet<u64> = index.usages().iter().map(|u| u.definition).collect();
        definitions.extend(found.datums.iter().map(|d| d.instance));
        definitions.extend(found.tolerances.iter().map(|t| t.target.instance));
        for dimension in &found.dimensions {
            definitions.insert(dimension.target.instance);
            definitions.extend(dimension.related.as_ref().map(|r| r.instance));
        }
        let mut resolved: BTreeMap<u64, BTreeMap<u64, Vec<StepKinds>>> = BTreeMap::new();
        let mut resolve = |id: u64| -> BTreeMap<u64, Vec<StepKinds>> {
            resolved
                .entry(id)
                .or_insert_with(|| {
                    index
                        .resolve(&graph, id)
                        .items
                        .into_iter()
                        .map(|item| (item.item, item.route_kinds))
                        .collect()
                })
                .clone()
        };
        let mut items: HashSet<u64> = HashSet::new();
        for &definition in &definitions {
            for (item, kinds) in resolve(definition) {
                assert!(graph.exchange().get(item).is_some());
                let back = index.naming(item);
                let naming = back.iter().find(|n| n.definition == definition);
                assert_eq!(
                    naming.map(|n| &n.route_kinds),
                    Some(&kinds),
                    "{}: #{definition} lists #{item}",
                    path.display()
                );
                items.insert(item);
            }
        }
        for &item in &items {
            for naming in index.naming(item) {
                assert_eq!(
                    resolve(naming.definition).get(&item),
                    Some(&naming.route_kinds),
                    "{}: naming(#{item}) lists #{}",
                    path.display(),
                    naming.definition
                );
            }
        }
    }
}
