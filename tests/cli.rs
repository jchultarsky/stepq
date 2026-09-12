//! Tests for the command-line binary.

#![cfg(feature = "cli")]

use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;

const SAMPLE: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('bracket.stp','2026-09-12T10:00:00',(''),(''),'','CAD 1.0','');
FILE_SCHEMA(('CONFIG_CONTROL_DESIGN'));
ENDSEC;
DATA;
#1=PRODUCT('b','bracket','',(#2));
#2=PRODUCT_CONTEXT('',#3,'mechanical');
#3=APPLICATION_CONTEXT('design');
#4=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
ENDSEC;
END-ISO-10303-21;
";

fn stepq() -> Command {
    Command::cargo_bin("stepq").unwrap()
}

#[test]
fn help_lists_subcommands() {
    stepq()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("split"))
        .stdout(predicate::str::contains("bom"));
}

#[test]
fn version_matches_cargo() {
    stepq()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn unimplemented_command_fails_cleanly() {
    stepq()
        .args(["tree", "assembly.stp"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not implemented"));
}

#[test]
fn info_table() {
    stepq()
        .args(["info", "-"])
        .write_stdin(SAMPLE)
        .assert()
        .success()
        .stdout(predicate::str::contains("<stdin>"))
        .stdout(predicate::str::contains("CONFIG_CONTROL_DESIGN"))
        .stdout(predicate::str::contains("CAD 1.0"))
        .stdout(predicate::str::contains("length millimetre"))
        .stdout(predicate::str::contains("4 (1 complex) in 1 data section"))
        .stdout(predicate::str::contains("1 (no assembly structure)"))
        .stdout(predicate::str::contains("Entity types (6)"))
        .stdout(predicate::str::contains("Warning").not());
}

#[test]
fn info_table_can_limit_entity_types() {
    stepq()
        .args(["info", "-", "--top", "2"])
        .write_stdin(SAMPLE)
        .assert()
        .success()
        .stdout(predicate::str::contains("… 4 more (--top 0 lists all)"));
}

#[test]
fn info_json() {
    let output = stepq()
        .args(["--format", "json", "info", "-"])
        .write_stdin(SAMPLE)
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["file"], "<stdin>");
    assert_eq!(json["instances"], 4);
    assert_eq!(json["products"], 1);
    assert_eq!(json["header"]["schemas"][0], "CONFIG_CONTROL_DESIGN");
    assert_eq!(json["units"]["length"][0], "millimetre");
    assert_eq!(json["entity_types"].as_array().unwrap().len(), 6);
}

#[test]
fn info_csv() {
    stepq()
        .args(["info", "-", "--format", "csv"])
        .write_stdin(SAMPLE)
        .assert()
        .success()
        .stdout(predicate::str::starts_with("entity_type,count\n"))
        .stdout(predicate::str::contains("\nPRODUCT,1\n"));
}

#[test]
fn info_warns_about_dangling_references() {
    stepq()
        .args(["info", "-"])
        .write_stdin(SAMPLE.replace(
            "#1=PRODUCT('b','bracket','',(#2));",
            "#1=PRODUCT('b','bracket','',(#99));",
        ))
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "1 references to undefined instances",
        ));
}

#[test]
fn info_reports_missing_files() {
    stepq()
        .args(["info", "no-such-file.stp"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("reading no-such-file.stp"));
}

#[test]
fn info_reports_syntax_errors_with_position() {
    stepq()
        .args(["info", "-"])
        .write_stdin("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=A(1,);\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains("parsing <stdin>"))
        .stderr(predicate::str::contains("line 5, column 8"));
}

#[test]
fn info_on_the_as1_assembly() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/steptools/as1-ug-214.stp");
    if !path.exists() {
        eprintln!(
            "skipping: {} not fetched (run tools/fetch-fixtures.sh)",
            path.display()
        );
        return;
    }
    let output = stepq()
        .args(["--format", "json", "info"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["products"], 9);
    assert_eq!(json["assembly_usages"], 13);
    assert_eq!(json["unresolved_references"], 0);
    assert_eq!(
        json["header"]["originating_system"],
        "UNIGRAPHICS SOLUTIONS - UNIGRAPHICS 16.0"
    );
}
