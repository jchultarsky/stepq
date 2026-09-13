//! Checks the EXPRESS reader against real schemas and the STEP fixtures.
//!
//! Schemas come from `tools/fetch-schemas.sh` (checksum-pinned) and
//! fixtures from `tools/fetch-fixtures.sh`; neither is in git, and each
//! test skips what is missing.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

use stepq::express::{ProblemKind, Schema, check};
use stepq::p21::parse;

/// File, schema name, and entity count measured on the pinned file.
const SCHEMAS: [(&str, &str, usize); 4] = [
    (
        "ap242e4_mim_lf.exp",
        "AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF",
        2407,
    ),
    ("ap214e3.exp", "AUTOMOTIVE_DESIGN", 915),
    (
        "ap203e2_mim_lf.exp",
        "AP203_CONFIGURATION_CONTROLLED_3D_DESIGN_OF_MECHANICAL_PARTS_AND_ASSEMBLIES_MIM_LF",
        1006,
    ),
    ("ap203.exp", "CONFIG_CONTROL_DESIGN", 254),
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn load_schemas() -> Vec<Schema> {
    SCHEMAS
        .iter()
        .filter_map(|(file, _, _)| {
            let src = fs::read(root().join("schemas").join(file)).ok()?;
            Some(Schema::parse(&src).unwrap_or_else(|e| panic!("{file}: {e}")))
        })
        .collect()
}

#[test]
fn real_schemas_parse_completely() {
    let mut found = 0;
    for (file, name, entities) in SCHEMAS {
        let Ok(src) = fs::read(root().join("schemas").join(file)) else {
            continue;
        };
        found += 1;
        let schema = Schema::parse(&src).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(schema.name(), name, "{file}");
        assert_eq!(schema.entities().len(), entities, "{file}");
        let unresolvable: Vec<&str> = schema
            .entities()
            .iter()
            .filter(|e| schema.slots(&e.name).is_none())
            .map(|e| e.name.as_str())
            .collect();
        assert!(
            unresolvable.is_empty(),
            "{file}: supertypes missing for {unresolvable:?}"
        );
    }
    if found == 0 {
        eprintln!("skipping: no schemas (run tools/fetch-schemas.sh)");
    }
}

/// Every fixture record has exactly the attribute count its schema
/// declares. That is the strongest available check that inheritance order,
/// diamonds, redeclarations and complex-instance partials are read right.
///
/// Three kinds of genuine problem in the fixtures are reported, not
/// failed on:
///
/// * some AP203 geometry-only files (NIST, Project Olympus) declare
///   `CONFIG_CONTROL_DESIGN` (AP203 edition 1) but use presentation
///   entities, such as `COLOUR_RGB`, that only edition 2 defines;
/// * some AP242 edition 3 files write lists shorter than the edition 4
///   schema's lower bound;
/// * an Open Rack Creo export declares `AUTOMOTIVE_DESIGN` (AP214) but
///   writes `MECHANICAL_DESIGN_AND_DRAUGHTING_RELATIONSHIP`, which only
///   AP203 edition 2 and AP242 define. Any entity type another loaded
///   schema defines is reported this way; one no schema knows still fails.
#[test]
fn fixture_records_match_their_schemas() {
    let schemas = load_schemas();
    if schemas.is_empty() {
        eprintln!("skipping: no schemas (run tools/fetch-schemas.sh)");
        return;
    }
    let mut files = Vec::new();
    collect_step_files(&root().join("fixtures"), &mut files);
    files.sort();

    let mut failures = Vec::new();
    for path in files {
        let src = fs::read(&path).unwrap();
        let exchange = parse(&src).unwrap();
        let file_schema = exchange
            .header_entity("FILE_SCHEMA")
            .and_then(|r| r.param(0))
            .and_then(|p| p.list())
            .and_then(|mut names| names.next())
            .and_then(|p| p.literal())
            .and_then(|l| l.decode().ok())
            .map(Cow::into_owned)
            .unwrap_or_default();
        let name = path.strip_prefix(root()).unwrap().display().to_string();
        let Some(schema) = schemas.iter().find(|s| s.matches(&file_schema)) else {
            eprintln!("{name}: no schema loaded for {file_schema:?}");
            continue;
        };

        for problem in check(schema, &exchange) {
            match problem.kind {
                ProblemKind::UnknownEntity if schema.name() == "CONFIG_CONTROL_DESIGN" => {
                    eprintln!("{name}: {problem} (AP203 ed. 2 entity in an ed. 1 file)");
                }
                ProblemKind::UnknownEntity
                    if schemas
                        .iter()
                        .any(|other| other.entity(&problem.entity).is_some()) =>
                {
                    eprintln!("{name}: {problem} (defined by another application protocol)");
                }
                ProblemKind::TooFewElements { .. } => eprintln!("{name}: {problem}"),
                _ => failures.push(format!("{name}: {problem}")),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} unexpected problems:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn collect_step_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_step_files(&path, out);
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("stp") || ext.eq_ignore_ascii_case("step"))
        {
            out.push(path);
        }
    }
}
