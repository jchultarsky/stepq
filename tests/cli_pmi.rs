//! Tests for `stepq pmi`.

#![cfg(feature = "cli")]

use assert_cmd::Command;
use predicates::prelude::*;

/// Datums A and B (B with a modifier), a position tolerance to them and a
/// flatness tolerance, a toleranced diameter and a location.
const SAMPLE: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('AP242'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('');
#2=PRODUCT_DEFINITION_CONTEXT('',#1,'design');
#10=PRODUCT('P','part','',());
#11=PRODUCT_DEFINITION_FORMATION('','',#10);
#12=PRODUCT_DEFINITION('design','',#11,#2);
#20=PRODUCT_DEFINITION_SHAPE('','',#12);
#30=DATUM('',$,#20,.F.,'A');
#31=DATUM('',$,#20,.F.,'B');
#40=DATUM_REFERENCE_COMPARTMENT('',$,#20,.F.,#30,$);
#41=DATUM_REFERENCE_COMPARTMENT('',$,#20,.F.,#31,(.MAXIMUM_MATERIAL_REQUIREMENT.));
#50=DATUM_SYSTEM('DS',$,#20,.F.,(#40,#41));
#60=SHAPE_ASPECT('','hole',#20,.F.);
#61=SHAPE_ASPECT('','face',#20,.F.);
#70=(LENGTH_MEASURE_WITH_UNIT()MEASURE_REPRESENTATION_ITEM()MEASURE_WITH_UNIT(LENGTH_MEASURE(0.75),#99)REPRESENTATION_ITEM(''));
#80=(GEOMETRIC_TOLERANCE('Position.1','',#70,#60)GEOMETRIC_TOLERANCE_WITH_DATUM_REFERENCE((#50))GEOMETRIC_TOLERANCE_WITH_MODIFIERS((.MAXIMUM_MATERIAL_REQUIREMENT.))POSITION_TOLERANCE());
#81=FLATNESS_TOLERANCE('Flatness.1','',#82,#61);
#82=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(0.05),#99);
#90=DIMENSIONAL_SIZE(#60,'diameter');
#91=DIMENSIONAL_CHARACTERISTIC_REPRESENTATION(#90,#92);
#92=SHAPE_DIMENSION_REPRESENTATION('',(#93),#1);
#93=(LENGTH_MEASURE_WITH_UNIT()MEASURE_REPRESENTATION_ITEM()MEASURE_WITH_UNIT(LENGTH_MEASURE(35.),#99)REPRESENTATION_ITEM('nominal value'));
#94=PLUS_MINUS_TOLERANCE(#95,#90);
#95=TOLERANCE_VALUE(#96,#97);
#96=(LENGTH_MEASURE_WITH_UNIT()MEASURE_REPRESENTATION_ITEM()MEASURE_WITH_UNIT(LENGTH_MEASURE(-0.2),#99)REPRESENTATION_ITEM(''));
#97=(LENGTH_MEASURE_WITH_UNIT()MEASURE_REPRESENTATION_ITEM()MEASURE_WITH_UNIT(LENGTH_MEASURE(0.),#99)REPRESENTATION_ITEM(''));
#98=DIMENSIONAL_LOCATION('linear distance',$,#60,#61);
#99=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
ENDSEC;END-ISO-10303-21;";

fn stepq() -> Command {
    Command::cargo_bin("stepq").unwrap()
}

#[test]
fn pmi_table_groups_by_product_definition() {
    stepq()
        .args(["pmi", "-"])
        .write_stdin(SAMPLE)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "part [P]  #12\n  \
             datum       A\n  \
             datum       B\n  \
             tolerance   position 0.75 (maximum_material_requirement) | A | B (maximum_material_requirement)  Position.1  [on SHAPE_ASPECT #60]\n  \
             tolerance   flatness 0.05  Flatness.1  [on SHAPE_ASPECT #61]\n  \
             dimension   diameter 35. (-0.2 .. 0.)  [on SHAPE_ASPECT #60]\n  \
             dimension   linear distance  [from SHAPE_ASPECT #60 to SHAPE_ASPECT #61]\n\
             \n\
             2 datums, 2 tolerances, 2 dimensions\n",
        ));
}

#[test]
fn pmi_json_and_csv() {
    let output = stepq()
        .args(["--format", "json", "pmi", "-"])
        .write_stdin(SAMPLE)
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["file"], "<stdin>");
    assert_eq!(json["datums"][0]["label"], "A");
    let position = &json["tolerances"][0];
    assert_eq!(position["kind"], "position");
    assert_eq!(position["magnitude"]["value"], "0.75");
    assert_eq!(position["datums"][1]["label"], "B");
    assert_eq!(
        position["datums"][1]["modifiers"][0],
        "maximum_material_requirement"
    );
    assert_eq!(position["target"]["product_definition"], 12);
    let diameter = &json["dimensions"][0];
    assert_eq!(diameter["kind"], "size");
    assert_eq!(diameter["values"][0]["value"], "35.");
    assert_eq!(diameter["tolerance"]["lower"]["value"], "-0.2");

    stepq()
        .args(["pmi", "-", "--format", "csv"])
        .write_stdin(SAMPLE)
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "category,product_definition,product,instance,type,name,value,lower,upper,datums,modifiers,target,target_type\n\
             datum,#12,part [P],#30,,A,,,,,,#30,DATUM\n",
        ))
        .stdout(predicate::str::contains(
            "tolerance,#12,part [P],#80,position,Position.1,0.75,,,A|B(maximum_material_requirement),maximum_material_requirement,#60,SHAPE_ASPECT\n",
        ))
        .stdout(predicate::str::contains(
            "dimension,#12,part [P],#90,size,diameter,35.,-0.2,0.,,,#60,SHAPE_ASPECT\n",
        ));
}

#[test]
fn a_file_without_pmi_says_so() {
    stepq()
        .args(["pmi", "-"])
        .write_stdin("ISO-10303-21;HEADER;FILE_SCHEMA(('AP242'));ENDSEC;DATA;#1=APPLICATION_CONTEXT('');ENDSEC;END-ISO-10303-21;")
        .assert()
        .success()
        .stdout(predicate::str::diff("no semantic PMI\n"));
}
