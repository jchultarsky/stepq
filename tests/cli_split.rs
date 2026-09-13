//! Tests for `stepq split`.

#![cfg(feature = "cli")]

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;

/// Assembly A uses part C twice; a category lists both products.
const ASSEMBLY: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('AP214'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('design');
#2=APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2001,#1);
#3=PRODUCT_CONTEXT('',#1,'mechanical');
#4=PRODUCT_DEFINITION_CONTEXT('part definition',#1,'design');
#11=PRODUCT('A','assembly','',(#3));
#12=PRODUCT_DEFINITION_FORMATION('1','',#11);
#10=PRODUCT_DEFINITION('a','',#12,#4);
#31=PRODUCT('C/1','bolt','',(#3));
#32=PRODUCT_DEFINITION_FORMATION('1','',#31);
#30=PRODUCT_DEFINITION('c','',#32,#4);
#40=PRODUCT_RELATED_PRODUCT_CATEGORY('part','',(#11,#31));
#100=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u1','C1','',#10,#30,$);
#101=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u2','C2','',#10,#30,$);
ENDSEC;END-ISO-10303-21;";

/// A fresh, empty directory for one test.
fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("stepq-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn stepq() -> Command {
    Command::cargo_bin("stepq").unwrap()
}

#[test]
fn split_writes_one_self_contained_file_per_definition() {
    let dir = scratch("split-files");
    stepq()
        .args(["split", "-", "--format", "csv", "--out"])
        .arg(&dir)
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "file,type,definition,product_id,product_name,instances,pruned\n\
             A.stp,assembly,10,A,assembly,13,1\n\
             C_1.stp,part,30,C/1,bolt,8,1\n",
        ));

    let assembly = fs::read_to_string(dir.join("A.stp")).unwrap();
    assert!(assembly.contains("PRODUCT('A'"));
    assert!(assembly.contains("PRODUCT('C/1'"));
    assert_eq!(
        assembly.matches("NEXT_ASSEMBLY_USAGE_OCCURRENCE").count(),
        2
    );

    let part = fs::read_to_string(dir.join("C_1.stp")).unwrap();
    assert!(part.contains("PRODUCT('C/1'"));
    assert!(!part.contains("PRODUCT('A'"), "the parent is not copied");
    assert!(!part.contains("NEXT_ASSEMBLY_USAGE_OCCURRENCE"));
    assert!(
        part.contains("PRODUCT_RELATED_PRODUCT_CATEGORY('part','',(#"),
        "the shared category is kept, filtered"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn split_refuses_to_overwrite_without_force() {
    let dir = scratch("split-force");
    stepq()
        .args(["split", "-", "--out"])
        .arg(&dir)
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote 2 files to"));
    stepq()
        .args(["split", "-", "--out"])
        .arg(&dir)
        .write_stdin(ASSEMBLY)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "already exists; pass --force to overwrite it",
        ));
    stepq()
        .args(["split", "-", "--force", "--out"])
        .arg(&dir)
        .write_stdin(ASSEMBLY)
        .assert()
        .success();
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn split_reports_orphans() {
    let dir = scratch("split-orphans");
    let with_orphan = ASSEMBLY.replace(
        "ENDSEC;END",
        "#900=DRAUGHTING_PRE_DEFINED_COLOUR('red');\nENDSEC;END",
    );
    stepq()
        .args(["split", "-", "--report-orphans", "--out"])
        .arg(&dir)
        .write_stdin(with_orphan)
        .assert()
        .success()
        .stdout(predicate::str::contains("\n1 instance is in no output\n"))
        .stdout(predicate::str::contains("DRAUGHTING_PRE_DEFINED_COLOUR"));
    fs::remove_dir_all(&dir).unwrap();

    let with_orphans = ASSEMBLY.replace(
        "ENDSEC;END",
        "#900=DRAUGHTING_PRE_DEFINED_COLOUR('red');\n\
         #901=DRAUGHTING_PRE_DEFINED_COLOUR('blue');\nENDSEC;END",
    );
    stepq()
        .args(["split", "-", "--report-orphans", "--out"])
        .arg(&dir)
        .write_stdin(with_orphans)
        .assert()
        .success()
        .stdout(predicate::str::contains("\n2 instances are in no output\n"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn split_bodies_checks_body_files_before_writing_anything() {
    let dir = scratch("split-bodies-exists");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("P.body-2.stp"), "keep me").unwrap();
    stepq()
        .args(["split", "-", "--bodies", "--out"])
        .arg(&dir)
        .write_stdin(BODIES)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "P.body-2.stp already exists; pass --force to overwrite it",
        ));
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["P.body-2.stp"], "nothing else is written");
    assert_eq!(
        fs::read_to_string(dir.join("P.body-2.stp")).unwrap(),
        "keep me"
    );

    stepq()
        .args(["split", "-", "--bodies", "--force", "--out"])
        .arg(&dir)
        .write_stdin(BODIES)
        .assert()
        .success();
    assert!(dir.join("P.body-1.stp").exists());
    assert!(
        fs::read_to_string(dir.join("P.body-2.stp"))
            .unwrap()
            .contains("MANIFOLD_SOLID_BREP('right'")
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// Part P with two solids in one shape representation, a colour on the
/// second, and a layer holding both.
const BODIES: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('AP214'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('design');
#2=APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2001,#1);
#3=PRODUCT_CONTEXT('',#1,'mechanical');
#4=PRODUCT_DEFINITION_CONTEXT('part definition',#1,'design');
#5=GEOMETRIC_REPRESENTATION_CONTEXT(3);
#11=PRODUCT('P','plate','',(#3));
#12=PRODUCT_DEFINITION_FORMATION('1','',#11);
#10=PRODUCT_DEFINITION('p','',#12,#4);
#20=PRODUCT_DEFINITION_SHAPE('','',#10);
#21=SHAPE_DEFINITION_REPRESENTATION(#20,#22);
#22=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#30,#40),#5);
#30=MANIFOLD_SOLID_BREP('left',#31);
#31=CLOSED_SHELL('',(#32));
#32=ADVANCED_FACE('',(),#33,.T.);
#33=PLANE('',#34);
#34=AXIS2_PLACEMENT_3D('',#35,$,$);
#35=CARTESIAN_POINT('',(0.,0.,0.));
#40=MANIFOLD_SOLID_BREP('right',#41);
#41=CLOSED_SHELL('',(#42));
#42=ADVANCED_FACE('',(),#43,.T.);
#43=PLANE('',#34);
#50=STYLED_ITEM('',(#51),#40);
#51=PRESENTATION_STYLE_ASSIGNMENT(());
#60=PRESENTATION_LAYER_ASSIGNMENT('layer 1','',(#30,#40));
ENDSEC;END-ISO-10303-21;";

#[test]
fn split_bodies_writes_one_file_per_solid() {
    let dir = scratch("split-bodies");
    stepq()
        .args(["split", "-", "--bodies", "--format", "csv", "--out"])
        .arg(&dir)
        .write_stdin(BODIES)
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "file,type,definition,product_id,product_name,instances,pruned\n\
             P.stp,part,10,P,plate,",
        ))
        .stdout(predicate::str::contains("\nP.body-1.stp,body,10,P,plate,"))
        .stdout(predicate::str::contains("\nP.body-2.stp,body,10,P,plate,"));

    let part = fs::read_to_string(dir.join("P.stp")).unwrap();
    assert_eq!(part.matches("MANIFOLD_SOLID_BREP").count(), 2);

    let left = fs::read_to_string(dir.join("P.body-1.stp")).unwrap();
    assert!(left.contains("MANIFOLD_SOLID_BREP('left'"), "{left}");
    assert!(!left.contains("'right'"), "{left}");
    assert!(
        !left.contains("STYLED_ITEM"),
        "the colour belongs to the other solid"
    );
    assert_eq!(left.matches("PLANE(").count(), 1, "{left}");
    assert!(
        left.contains("ADVANCED_BREP_SHAPE_REPRESENTATION('',(#"),
        "{left}"
    );
    assert_eq!(
        left.matches("PRESENTATION_LAYER_ASSIGNMENT('layer 1','',(#")
            .count(),
        1
    );

    let right = fs::read_to_string(dir.join("P.body-2.stp")).unwrap();
    assert!(right.contains("MANIFOLD_SOLID_BREP('right'"), "{right}");
    assert!(!right.contains("'left'"), "{right}");
    assert!(right.contains("STYLED_ITEM"), "its colour comes with it");
    // Every output reads back with no dangling reference.
    for text in [&part, &left, &right] {
        stepq()
            .args(["lint", "-"])
            .write_stdin(text.as_str())
            .assert()
            .stdout(predicate::str::contains("dangling-reference").not());
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn split_refuses_dangling_references() {
    let dir = scratch("split-dangling");
    stepq()
        .args(["split", "-", "--out"])
        .arg(&dir)
        .write_stdin(ASSEMBLY.replace("(#11,#31)", "(#11,#99)"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("dangling reference"));
    assert!(!dir.exists(), "nothing is written");
}
