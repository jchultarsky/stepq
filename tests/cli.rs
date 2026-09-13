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
fn unknown_command_fails_cleanly() {
    stepq()
        .args(["frobnicate", "assembly.stp"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
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

/// Assembly A: two of sub-assembly B (each with three C) and a quantified
/// usage of four D. B's first usage is placed normally; C's first usage
/// names the parent shape as `rep_1`.
const ASSEMBLY: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('AP242'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('design');
#2=PRODUCT_CONTEXT('',#1,'mechanical');
#3=PRODUCT_DEFINITION_CONTEXT('part definition',#1,'design');
#11=PRODUCT('A','assembly','',(#2));
#12=PRODUCT_DEFINITION_FORMATION('1','',#11);
#10=PRODUCT_DEFINITION('a-def','',#12,#3);
#21=PRODUCT('B','bracket assembly','',(#2));
#22=PRODUCT_DEFINITION_FORMATION_WITH_SPECIFIED_SOURCE('2','',#21,.MADE.);
#20=PRODUCT_DEFINITION('b-def','',#22,#3);
#31=PRODUCT('C','bolt','',(#2));
#32=PRODUCT_DEFINITION_FORMATION('1','',#31);
#30=PRODUCT_DEFINITION('c-def','',#32,#3);
#41=PRODUCT('D','plate','',(#2));
#42=PRODUCT_DEFINITION_FORMATION('1','',#41);
#40=PRODUCT_DEFINITION('d-def','',#42,#3);
#50=SHAPE_REPRESENTATION('a',(#60),#1);
#51=SHAPE_REPRESENTATION('b',(#60),#1);
#52=SHAPE_REPRESENTATION('c',(#60),#1);
#60=AXIS2_PLACEMENT_3D('',#61,$,$);
#61=CARTESIAN_POINT('',(0.,0.,0.));
#70=PRODUCT_DEFINITION_SHAPE('','',#10);
#71=PRODUCT_DEFINITION_SHAPE('','',#20);
#72=PRODUCT_DEFINITION_SHAPE('','',#30);
#80=SHAPE_DEFINITION_REPRESENTATION(#70,#50);
#81=SHAPE_DEFINITION_REPRESENTATION(#71,#51);
#82=SHAPE_DEFINITION_REPRESENTATION(#72,#52);
#100=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u1','B1','',#10,#20,$);
#101=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u2','B2','',#10,#20,$);
#102=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u3','C1','',#20,#30,'X1');
#103=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u4','C2','',#20,#30,'X2');
#104=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u5','C3','',#20,#30,'X3');
#105=(ASSEMBLY_COMPONENT_USAGE($)NEXT_ASSEMBLY_USAGE_OCCURRENCE()PRODUCT_DEFINITION_RELATIONSHIP('u6','D','',#10,#40)PRODUCT_DEFINITION_USAGE()QUANTIFIED_ASSEMBLY_COMPONENT_USAGE(#106));
#106=MEASURE_WITH_UNIT(COUNT_MEASURE(4.),#1);
#90=PRODUCT_DEFINITION_SHAPE('','',#100);
#91=(REPRESENTATION_RELATIONSHIP('','',#51,#50)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#92)SHAPE_REPRESENTATION_RELATIONSHIP());
#92=ITEM_DEFINED_TRANSFORMATION('','',#60,#60);
#93=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#91,#90);
#94=PRODUCT_DEFINITION_SHAPE('','',#102);
#95=REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION('','',#51,#52,#92);
#96=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#95,#94);
ENDSEC;END-ISO-10303-21;";

#[test]
fn tree_groups_repeated_components() {
    stepq()
        .args(["tree", "-"])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "assembly [A]\n\
             ├── bracket assembly [B]  ×2\n\
             │   └── bolt [C]  ×3\n\
             └── plate [D]  ×4\n",
        ))
        .stdout(predicate::str::contains(
            "4 product definitions, 6 assembly usages, 1 top-level",
        ))
        .stdout(predicate::str::contains(
            "note: 1 placements name the assembly's shape as rep_1",
        ));
}

#[test]
fn tree_lists_usages_with_their_placement() {
    stepq()
        .args(["tree", "-", "--usages"])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "├── bracket assembly [B]  #100 'B1'  placed by #93 → #91 (#92)\n",
        ))
        .stdout(predicate::str::contains(
            "bolt [C]  #102 'C1' X1  placed by #96 → #95 (#92) [rep_1/rep_2 reversed]\n",
        ))
        .stdout(predicate::str::contains("#101 'B2'  (no placement)"))
        .stdout(predicate::str::contains(
            "└── plate [D]  #105 'D' ×4  (no placement)",
        ));
}

#[test]
fn tree_csv_has_one_row_per_usage() {
    let output = stepq()
        .args(["--format", "csv", "tree", "-"])
        .write_stdin(ASSEMBLY)
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows.len(), 7);
    assert!(rows[0].starts_with("usage,usage_id,parent,"));
    assert_eq!(
        rows[3],
        "102,u3,20,bracket assembly [B],30,bolt [C],1,X1,shape_relationship,95,92,true"
    );
}

#[test]
fn tree_json_has_definitions_usages_and_roots() {
    let output = stepq()
        .args(["--format", "json", "tree", "-"])
        .write_stdin(ASSEMBLY)
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["roots"], serde_json::json!([0]));
    assert_eq!(json["definitions"][0]["product"]["name"], "assembly");
    assert_eq!(json["usages"].as_array().unwrap().len(), 6);
    assert_eq!(
        json["usages"][5]["kind"],
        "quantified_assembly_component_usage"
    );
    assert_eq!(json["usages"][2]["placement"]["kind"], "shape_relationship");
    assert_eq!(json["usages"][2]["placement"]["reversed"], true);
    assert_eq!(json["usages"][1]["placement"], serde_json::Value::Null);
}

const BOM_SUMMARY: &str = "1 sub-assembly, 2 distinct parts, 10 parts in total\n";

#[test]
fn bom_is_a_tree_by_default() {
    stepq()
        .args(["bom", "-", "--charset", "utf8"])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::diff(format!(
            "assembly [A]\n\
             ├── bracket assembly [B]  ×2\n\
             │   └── bolt [C]  ×3  (6 total)\n\
             └── plate [D]  ×4\n\
             \n{BOM_SUMMARY}"
        )));
}

#[test]
fn bom_tree_in_ascii() {
    stepq()
        .args(["bom", "-", "--format", "tree", "--charset", "ascii"])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::diff(format!(
            "assembly [A]\n\
             |-- bracket assembly [B]  x2\n\
             |   `-- bolt [C]  x3  (6 total)\n\
             `-- plate [D]  x4\n\
             \n{BOM_SUMMARY}"
        )));
}

#[test]
fn bom_charset_follows_the_locale() {
    // Windows ignores the locale variables and always draws UTF-8.
    let ascii = if cfg!(windows) {
        "├── bracket assembly [B]"
    } else {
        "|-- bracket assembly [B]"
    };
    stepq()
        .args(["bom", "-"])
        .env("LC_ALL", "C")
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::contains(ascii));
    stepq()
        .args(["bom", "-"])
        .env("LC_ALL", "en_US.UTF-8")
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::contains("├── bracket assembly [B]"));
}

#[test]
fn bom_tree_prefix_and_depth() {
    stepq()
        .args(["bom", "-", "--prefix", "depth", "--charset", "utf8"])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::diff(format!(
            "0 assembly [A]\n\
             1 bracket assembly [B]  ×2\n\
             2 bolt [C]  ×3  (6 total)\n\
             1 plate [D]  ×4\n\
             \n{BOM_SUMMARY}"
        )));
    stepq()
        .args(["bom", "-", "--depth", "1", "--charset", "utf8"])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::diff(format!(
            "assembly [A]\n\
             ├── bracket assembly [B]  ×2\n\
             └── plate [D]  ×4\n\
             \n{BOM_SUMMARY}"
        )));
}

/// A uses B and E; E uses B too; B uses part C.
const REPEATED: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('X'));ENDSEC;DATA;
#1=PRODUCT_DEFINITION('a','',$,$);
#2=PRODUCT_DEFINITION('b','',$,$);
#3=PRODUCT_DEFINITION('e','',$,$);
#4=PRODUCT_DEFINITION('c','',$,$);
#10=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','','',#1,#2,$);
#11=NEXT_ASSEMBLY_USAGE_OCCURRENCE('2','','',#1,#3,$);
#12=NEXT_ASSEMBLY_USAGE_OCCURRENCE('3','','',#3,#2,$);
#13=NEXT_ASSEMBLY_USAGE_OCCURRENCE('4','','',#2,#4,$);
ENDSEC;END-ISO-10303-21;";

#[test]
fn bom_tree_marks_repeated_sub_assemblies() {
    stepq()
        .args(["bom", "-", "--charset", "utf8"])
        .write_stdin(REPEATED)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "a\n\
             ├── b\n\
             │   └── c\n\
             └── e\n    \
             └── b (*)\n\
             \n\
             2 sub-assemblies, 1 distinct part, 2 parts in total\n\
             (*) listed in full above; use --no-dedupe to repeat it\n",
        ));
    stepq()
        .args(["bom", "-", "--charset", "utf8", "--no-dedupe"])
        .write_stdin(REPEATED)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "a\n\
             ├── b\n\
             │   └── c\n\
             └── e\n    \
             └── b\n        \
             └── c\n\
             \n\
             2 sub-assemblies, 1 distinct part, 2 parts in total\n",
        ));
}

#[test]
fn bom_csv_is_an_indented_bom() {
    stepq()
        .args(["bom", "-", "--format", "csv"])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "level,item,definition,product_id,product_name,type,quantity,total_quantity\n\
             0,,10,A,assembly,assembly,1,1\n\
             1,1,20,B,bracket assembly,assembly,2,2\n\
             2,1.1,30,C,bolt,part,3,6\n\
             1,2,40,D,plate,part,4,4\n",
        ));
}

#[test]
fn bom_json_is_nested() {
    let output = stepq()
        .args(["--format", "json", "bom", "-"])
        .write_stdin(ASSEMBLY)
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let root = &json[0];
    assert_eq!(root["item"], "");
    assert_eq!(root["definition"]["product"]["id"], "A");
    let bolt = &root["components"][0]["components"][0];
    assert_eq!(bolt["item"], "1.1");
    assert_eq!(bolt["level"], 2);
    assert_eq!(bolt["quantity"], 3.0);
    assert_eq!(bolt["total_quantity"], 6.0);
    assert_eq!(bolt["kind"], "part");
    assert_eq!(bolt["components"], serde_json::json!([]));
    assert!(bolt.get("truncated").is_none());
}

#[test]
fn bom_flat_lists_total_quantities() {
    stepq()
        .args(["bom", "-", "--flat"])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "assembly [A]\n  \
             QUANTITY  TYPE      PRODUCT\n         \
             2  assembly  bracket assembly [B]\n         \
             6  part      bolt [C]\n         \
             4  part      plate [D]\n",
        ));
    stepq()
        .args(["bom", "-", "--flat", "--format", "csv"])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "root,product_id,product_name,type,quantity\n\
             assembly [A],B,bracket assembly,assembly,2\n\
             assembly [A],C,bolt,part,6\n\
             assembly [A],D,plate,part,4\n",
        ));
    let output = stepq()
        .args(["--format", "json", "bom", "-", "--flat"])
        .write_stdin(ASSEMBLY)
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json[0]["lines"][1]["quantity"], 6.0);
}
