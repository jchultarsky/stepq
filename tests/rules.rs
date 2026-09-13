//! Checks every closure rule's attribute position against the real schemas.
//!
//! The rules in `stepq::model::RULES` address attributes by position so
//! that no schema is needed at run time. A wrong position silently pulls in
//! the wrong instances, so each one is checked here against the long-form
//! schemas fetched by `tools/fetch-schemas.sh`. Skips when none are present.

use std::fs;
use std::path::Path;

use stepq::express::Schema;
use stepq::model::RULES;

const SCHEMA_FILES: [&str; 4] = [
    "ap242e4_mim_lf.exp",
    "ap214e3.exp",
    "ap203e2_mim_lf.exp",
    "ap203.exp",
];

#[test]
fn rule_positions_match_the_schemas() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/schemas");
    let schemas: Vec<Schema> = SCHEMA_FILES
        .iter()
        .filter_map(|file| fs::read(dir.join(file)).ok())
        .map(|src| Schema::parse(&src).unwrap())
        .collect();
    if schemas.is_empty() {
        eprintln!("skipping: no schemas (run tools/fetch-schemas.sh)");
        return;
    }

    let mut failures = Vec::new();
    let mut unknown = Vec::new();
    let mut checked = 0;
    for rule in RULES {
        for entity in rule.entities {
            let mut found = false;
            for schema in &schemas {
                let Some(slots) = schema.slots(entity) else {
                    continue;
                };
                found = true;
                checked += 1;
                match slots.get(rule.simple) {
                    Some(slot) if slot.name.eq_ignore_ascii_case(rule.attribute) => {}
                    other => failures.push(format!(
                        "{}: {entity} attribute {} is {:?}, rule expects {}",
                        schema.name(),
                        rule.simple,
                        other.map(|slot| slot.name.as_str()),
                        rule.attribute
                    )),
                }
            }
            if !found {
                unknown.push(*entity);
            }
        }
        if let Some((partial, index)) = rule.partial {
            for schema in &schemas {
                let Some(own) = schema.complex_slots(&[partial]) else {
                    continue;
                };
                checked += 1;
                match own[0].get(index) {
                    Some(slot) if slot.name.eq_ignore_ascii_case(rule.attribute) => {}
                    other => failures.push(format!(
                        "{}: partial {partial} attribute {index} is {:?}, rule expects {}",
                        schema.name(),
                        other.map(|slot| slot.name.as_str()),
                        rule.attribute
                    )),
                }
            }
        }
    }

    eprintln!(
        "checked {checked} rule positions against {} schemas",
        schemas.len()
    );
    assert!(
        failures.is_empty(),
        "{} wrong positions:\n{}",
        failures.len(),
        failures.join("\n")
    );
    if schemas.len() == SCHEMA_FILES.len() {
        assert!(
            unknown.is_empty(),
            "entities in no schema (typos?): {unknown:?}"
        );
    }
}
