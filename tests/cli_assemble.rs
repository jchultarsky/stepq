//! Tests for `stepq assemble`.

#![cfg(feature = "cli")]

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;

mod common;

/// Assembly A places part P twice; P's solid has a colour.
const ASSEMBLY: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('AUTOMOTIVE_DESIGN'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('design');
#2=APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2001,#1);
#3=PRODUCT_CONTEXT('',#1,'mechanical');
#4=PRODUCT_DEFINITION_CONTEXT('part definition',#1,'design');
#5=GEOMETRIC_REPRESENTATION_CONTEXT(3);
#6=GEOMETRIC_REPRESENTATION_CONTEXT(3);
#11=PRODUCT('A','assembly','',(#3));
#12=PRODUCT_DEFINITION_FORMATION('1','',#11);
#10=PRODUCT_DEFINITION('a','',#12,#4);
#13=PRODUCT_DEFINITION_SHAPE('','',#10);
#14=SHAPE_DEFINITION_REPRESENTATION(#13,#15);
#15=SHAPE_REPRESENTATION('',(#34,#36),#5);
#21=PRODUCT('P','plate','',(#3));
#22=PRODUCT_DEFINITION_FORMATION('1','',#21);
#20=PRODUCT_DEFINITION('p','',#22,#4);
#23=PRODUCT_DEFINITION_SHAPE('','',#20);
#24=SHAPE_DEFINITION_REPRESENTATION(#23,#25);
#25=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#38,#30),#6);
#30=MANIFOLD_SOLID_BREP('plate',#31);
#31=CLOSED_SHELL('',(#32));
#32=ADVANCED_FACE('',(),#33,.T.);
#33=PLANE('',#38);
#34=AXIS2_PLACEMENT_3D('',#35,$,$);
#35=CARTESIAN_POINT('',(0.,0.,0.));
#36=AXIS2_PLACEMENT_3D('',#37,$,$);
#37=CARTESIAN_POINT('',(10.,0.,0.));
#38=AXIS2_PLACEMENT_3D('',#39,$,$);
#39=CARTESIAN_POINT('',(0.,0.,0.));
#40=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u1','P1','',#10,#20,$);
#41=PRODUCT_DEFINITION_SHAPE('','',#40);
#42=(REPRESENTATION_RELATIONSHIP('','',#25,#15)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#43)SHAPE_REPRESENTATION_RELATIONSHIP());
#43=ITEM_DEFINED_TRANSFORMATION('','',#38,#34);
#44=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#42,#41);
#45=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u2','P2','',#10,#20,$);
#46=PRODUCT_DEFINITION_SHAPE('','',#45);
#47=(REPRESENTATION_RELATIONSHIP('','',#25,#15)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#48)SHAPE_REPRESENTATION_RELATIONSHIP());
#48=ITEM_DEFINED_TRANSFORMATION('','',#38,#36);
#49=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#47,#46);
#50=STYLED_ITEM('',(#51),#30);
#51=PRESENTATION_STYLE_ASSIGNMENT(());
ENDSEC;END-ISO-10303-21;";

fn stepq() -> Command {
    Command::cargo_bin("stepq").unwrap()
}

#[test]
fn assemble_undoes_split_master() {
    let dir = std::env::temp_dir().join(format!("stepq-assemble-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let original = dir.join("original.stp");
    fs::write(&original, ASSEMBLY).unwrap();
    let parts = dir.join("parts");
    stepq()
        .args(["split", "--master", "--out"])
        .arg(&parts)
        .arg(&original)
        .assert()
        .success();

    let assembled = dir.join("assembled.stp");
    stepq()
        .arg("assemble")
        .arg(parts.join("A.stp"))
        .arg("-o")
        .arg(&assembled)
        .assert()
        .success()
        .stdout(predicate::str::contains("merged 1 file\n  P.stp\n"));

    let text = fs::read_to_string(&assembled).unwrap();
    assert!(!text.contains("DOCUMENT_FILE"), "{text}");
    assert_eq!(text.matches("MANIFOLD_SOLID_BREP").count(), 1, "{text}");
    stepq().args(["lint"]).arg(&assembled).assert().success();
    stepq()
        .args(["diff", "--section", "products", "--section", "components"])
        .arg(&original)
        .arg(&assembled)
        .assert()
        .success();

    // The output is not overwritten without --force.
    stepq()
        .arg("assemble")
        .arg(parts.join("A.stp"))
        .arg("-o")
        .arg(&assembled)
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));
    fs::remove_dir_all(&dir).unwrap();
}

/// The AS1 assembly from every exporter fetched: split into masters and
/// assembled again, it has the original products and component quantities.
/// Its nut is used by two sub-assemblies and must come back once.
#[test]
fn assembling_split_masters_of_as1_restores_the_structure() {
    let files: Vec<_> = common::step_files()
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().starts_with("as1"))
        })
        .collect();
    for original in files {
        let name = original.file_stem().unwrap().to_string_lossy().into_owned();
        let dir =
            std::env::temp_dir().join(format!("stepq-assemble-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        stepq()
            .args(["split", "--master", "--out"])
            .arg(&dir)
            .arg(&original)
            .assert()
            .success();

        // The top master is the one no other file refers to.
        let masters: Vec<(String, String)> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    fs::read_to_string(entry.path()).unwrap(),
                )
            })
            .filter(|(_, text)| text.contains("DOCUMENT_FILE("))
            .collect();
        let referred = |file: &str| {
            let reference = format!("DOCUMENT_FILE('{file}'");
            masters.iter().any(|(_, text)| text.contains(&reference))
        };
        let top: Vec<_> = masters.iter().filter(|(file, _)| !referred(file)).collect();
        assert_eq!(top.len(), 1, "{name}: top masters {top:?}");

        let assembled = dir.join("assembled.step");
        stepq()
            .arg("assemble")
            .arg(dir.join(&top[0].0))
            .arg("-o")
            .arg(&assembled)
            .assert()
            .success();
        stepq()
            .args(["diff", "--section", "products", "--section", "components"])
            .arg(&original)
            .arg(&assembled)
            .assert()
            .success();
        fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn a_missing_component_file_is_reported() {
    let dir = std::env::temp_dir().join(format!("stepq-assemble-missing-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    stepq()
        .args(["split", "-", "--master", "--out"])
        .arg(&dir)
        .write_stdin(ASSEMBLY)
        .assert()
        .success();
    fs::remove_file(dir.join("P.stp")).unwrap();
    stepq()
        .arg("assemble")
        .arg(dir.join("A.stp"))
        .args(["-o", "-"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("external reference to P.stp"));
    fs::remove_dir_all(&dir).unwrap();
}
