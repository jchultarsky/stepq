//! Tests for `stepq split --master`.

#![cfg(feature = "cli")]

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;

/// Assembly A places part P, whose solid has a colour; both shapes share
/// the placement #34.
const ASSEMBLY: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('AUTOMOTIVE_DESIGN'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('design');
#2=APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2001,#1);
#3=PRODUCT_CONTEXT('',#1,'mechanical');
#4=PRODUCT_DEFINITION_CONTEXT('part definition',#1,'design');
#5=GEOMETRIC_REPRESENTATION_CONTEXT(3);
#11=PRODUCT('A','assembly','',(#3));
#12=PRODUCT_DEFINITION_FORMATION('1','',#11);
#10=PRODUCT_DEFINITION('a','',#12,#4);
#13=PRODUCT_DEFINITION_SHAPE('','',#10);
#14=SHAPE_DEFINITION_REPRESENTATION(#13,#15);
#15=SHAPE_REPRESENTATION('',(#34),#5);
#21=PRODUCT('P','plate','',(#3));
#22=PRODUCT_DEFINITION_FORMATION('1','',#21);
#20=PRODUCT_DEFINITION('p','',#22,#4);
#23=PRODUCT_DEFINITION_SHAPE('','',#20);
#24=SHAPE_DEFINITION_REPRESENTATION(#23,#25);
#25=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#34,#30),#5);
#30=MANIFOLD_SOLID_BREP('plate',#31);
#31=CLOSED_SHELL('',(#32));
#32=ADVANCED_FACE('',(),#33,.T.);
#33=PLANE('',#34);
#34=AXIS2_PLACEMENT_3D('',#35,$,$);
#35=CARTESIAN_POINT('',(0.,0.,0.));
#40=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u1','P1','',#10,#20,$);
#41=PRODUCT_DEFINITION_SHAPE('','',#40);
#42=(REPRESENTATION_RELATIONSHIP('','',#25,#15)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#43)SHAPE_REPRESENTATION_RELATIONSHIP());
#43=ITEM_DEFINED_TRANSFORMATION('','',#34,#34);
#44=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#42,#41);
#50=STYLED_ITEM('',(#51),#30);
#51=PRESENTATION_STYLE_ASSIGNMENT(());
ENDSEC;END-ISO-10303-21;";

fn stepq() -> Command {
    Command::cargo_bin("stepq").unwrap()
}

#[test]
fn split_master_writes_assemblies_that_refer_to_component_files() {
    let dir = std::env::temp_dir().join(format!("stepq-split-master-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    stepq()
        .args(["split", "-", "--master", "--format", "csv", "--out"])
        .arg(&dir)
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::contains("\nA.stp,master,10,A,assembly,"))
        .stdout(predicate::str::contains("\nP.stp,part,20,P,plate,"));

    let master = fs::read_to_string(dir.join("A.stp")).unwrap();
    for expected in [
        // The component keeps its product, definition and placements …
        "#21=PRODUCT('P','plate','',(#3));",
        "#20=PRODUCT_DEFINITION('p','',#22,#4);",
        "#24=SHAPE_DEFINITION_REPRESENTATION(#23,#25);",
        "#25=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#34),#5);",
        "#40=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u1','P1','',#10,#20,$);",
        "#44=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#42,#41);",
        // … and refers to its file.
        "#52=DOCUMENT_TYPE('geometry');",
        "#53=DOCUMENT_FILE('P.stp','',$,#52,'',$);",
        "#54=DOCUMENT_REPRESENTATION_TYPE('digital',#53);",
        "#55=IDENTIFICATION_ROLE('external document id and location',$);",
        "#56=EXTERNAL_SOURCE(IDENTIFIER(''));",
        "#57=APPLIED_EXTERNAL_IDENTIFICATION_ASSIGNMENT('P.stp',#55,#56,(#53));",
        "#58=APPLIED_DOCUMENT_REFERENCE(#53,'',(#20));",
        "#60=ROLE_ASSOCIATION(#59,#58);",
        "#61=PROPERTY_DEFINITION('external definition',$,#53);",
        "#62=PROPERTY_DEFINITION_REPRESENTATION(#61,#25);",
    ] {
        assert!(master.contains(expected), "missing {expected}\n{master}");
    }
    for geometry in [
        "MANIFOLD_SOLID_BREP",
        "CLOSED_SHELL",
        "PLANE(",
        "STYLED_ITEM",
    ] {
        assert!(
            !master.contains(geometry),
            "{geometry} in the master\n{master}"
        );
    }

    let part = fs::read_to_string(dir.join("P.stp")).unwrap();
    assert!(part.contains("MANIFOLD_SOLID_BREP('plate'"));
    assert!(part.contains("STYLED_ITEM"));
    assert!(!part.contains("DOCUMENT_FILE"));

    for text in [&master, &part] {
        stepq()
            .args(["lint", "-"])
            .write_stdin(text.as_str())
            .assert()
            .stdout(predicate::str::contains("dangling-reference").not());
    }
    fs::remove_dir_all(&dir).unwrap();
}
