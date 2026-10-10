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
fn pmi_table_lists_datums_then_tolerances_then_dimensions() {
    // Datums defined after the tolerances and a dimension before them.
    let reordered = SAMPLE
        .replace("#30=DATUM(", "#130=DATUM(")
        .replace("#31=DATUM(", "#131=DATUM(")
        .replace(",#30,$);", ",#130,$);")
        .replace(",#31,(", ",#131,(")
        .replace("#98=DIMENSIONAL_LOCATION(", "#8=DIMENSIONAL_LOCATION(");
    stepq()
        .args(["pmi", "-"])
        .write_stdin(reordered.as_str())
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "part [P]  #12\n  \
             datum       A\n  \
             datum       B\n  \
             tolerance   position 0.75 (maximum_material_requirement) | A | B (maximum_material_requirement)  Position.1  [on SHAPE_ASPECT #60]\n  \
             tolerance   flatness 0.05  Flatness.1  [on SHAPE_ASPECT #61]\n  \
             dimension   linear distance  [from SHAPE_ASPECT #60 to SHAPE_ASPECT #61]\n  \
             dimension   diameter 35. (-0.2 .. 0.)  [on SHAPE_ASPECT #60]\n\
             \n\
             2 datums, 2 tolerances, 2 dimensions\n",
        ));
    let csv = stepq()
        .args(["pmi", "-", "--format", "csv"])
        .write_stdin(reordered)
        .output()
        .unwrap();
    let categories: Vec<String> = String::from_utf8(csv.stdout)
        .unwrap()
        .lines()
        .skip(1)
        .map(|line| {
            let fields: Vec<&str> = line.split(',').collect();
            format!("{} {}", fields[0], fields[3])
        })
        .collect();
    assert_eq!(
        categories,
        [
            "datum #130",
            "datum #131",
            "tolerance #80",
            "tolerance #81",
            "dimension #8",
            "dimension #90"
        ]
    );
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

/// The sample with faces named for the hole and the face, and for datum A's
/// feature.
fn with_usages() -> String {
    SAMPLE.replace(
        "ENDSEC;END-ISO-10303-21;",
        "#200=ADVANCED_FACE('',(),#209,.T.);#201=ADVANCED_FACE('',(),#209,.T.);
#202=ADVANCED_FACE('',(),#209,.T.);#203=SHAPE_REPRESENTATION('',(#200,#201,#202),#1);
#204=GEOMETRIC_ITEM_SPECIFIC_USAGE('','',#60,#203,#200);
#205=GEOMETRIC_ITEM_SPECIFIC_USAGE('','',#61,#203,#201);
#206=DATUM_FEATURE('','',#20,.T.);#207=SHAPE_ASPECT_RELATIONSHIP('','',#206,#30);
#208=GEOMETRIC_ITEM_SPECIFIC_USAGE('','',#206,#203,#202);
ENDSEC;END-ISO-10303-21;",
    )
}

#[test]
fn pmi_geometry_lists_what_each_stands_for() {
    stepq()
        .args(["pmi", "--geometry", "-"])
        .write_stdin(with_usages())
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "part [P]  #12\n  \
             datum       A\n              \
             ADVANCED_FACE #202\n  \
             datum       B\n              \
             no items; not resolved: #31 (DATUM): nothing names its items\n  \
             tolerance   position 0.75 (maximum_material_requirement) | A | B (maximum_material_requirement)  Position.1  [on SHAPE_ASPECT #60]\n              \
             ADVANCED_FACE #200\n  \
             tolerance   flatness 0.05  Flatness.1  [on SHAPE_ASPECT #61]\n              \
             ADVANCED_FACE #201\n  \
             dimension   diameter 35. (-0.2 .. 0.)  [on SHAPE_ASPECT #60]\n              \
             ADVANCED_FACE #200\n  \
             dimension   linear distance  [from SHAPE_ASPECT #60 to SHAPE_ASPECT #61]\n              \
             from: ADVANCED_FACE #200\n              \
             to: ADVANCED_FACE #201\n\
             \n\
             2 datums, 2 tolerances, 2 dimensions\n",
        ));
}

#[test]
fn pmi_geometry_json_and_csv() {
    let out = stepq()
        .args(["pmi", "--geometry", "--format", "json", "-"])
        .write_stdin(with_usages())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["datums"][0]["geometry"]["items"][0]["item"], 202);
    assert_eq!(
        json["datums"][0]["geometry"]["items"][0]["via"][0]["kind"],
        "datum_feature"
    );
    assert_eq!(
        json["datums"][1]["geometry"]["unresolved"][0]["reason"]["kind"],
        "no_usage"
    );
    let location = &json["dimensions"][1];
    assert_eq!(location["geometry"]["items"][0]["item"], 200);
    assert_eq!(location["related_geometry"]["items"][0]["item"], 201);
    assert!(json["tolerances"][0].get("related_geometry").is_none());

    stepq()
        .args(["pmi", "--geometry", "--format", "csv", "-"])
        .write_stdin(with_usages())
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "target,target_type,items,unresolved,related_items,related_unresolved\n",
        ))
        .stdout(predicate::str::contains(
            ",#31,DATUM,,#31 nothing names its items,,\n",
        ))
        .stdout(predicate::str::contains(",#60,SHAPE_ASPECT,#200,,#201,\n"));
}
