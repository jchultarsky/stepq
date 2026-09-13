//! Tests for the command-line binary.

#![cfg(feature = "cli")]

use std::fmt::Write as _;
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
fn lint_warnings_alone_exit_zero() {
    stepq()
        .args(["lint", "-"])
        .write_stdin(SAMPLE)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "<stdin>: schema checks skipped (pass --schema)\n\
             \n\
             SEVERITY  CHECK                           INSTANCE  MESSAGE\n\
             warning   missing-application-protocol              no APPLICATION_PROTOCOL_DEFINITION; readers cannot tell which protocol the file conforms to\n\
             warning   uncategorized-product                 #1  PRODUCT is in no PRODUCT_RELATED_PRODUCT_CATEGORY\n\
             \n\
             0 errors, 2 warnings\n",
        ));
}

/// Two representations placed by a transformation but sharing one context,
/// and a reference to an instance that does not exist.
const LINT_ERRORS: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('DEMO'));ENDSEC;DATA;
#1=APPLICATION_PROTOCOL_DEFINITION('','demo',2026,#2);
#2=APPLICATION_CONTEXT('');
#10=GEOMETRIC_REPRESENTATION_CONTEXT(3);
#20=SHAPE_REPRESENTATION('',(),#10);
#21=SHAPE_REPRESENTATION('',(),#10);
#30=REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION('','',#20,#21,#40);
#40=ITEM_DEFINED_TRANSFORMATION('','',#99,#99);
ENDSEC;END-ISO-10303-21;";

#[test]
fn lint_errors_exit_one() {
    stepq()
        .args(["lint", "-"])
        .write_stdin(LINT_ERRORS)
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "error     shared-context                       #30  representations #20 and #21 share context #10",
        ))
        .stdout(predicate::str::contains(
            "error     dangling-reference                   #40  references undefined instance #99",
        ))
        .stdout(predicate::str::ends_with("3 errors, 0 warnings\n"));
    stepq()
        .args(["lint", "-"])
        .write_stdin("ISO-10303-21;HEADER;ENDSEC;DATA;#1=A(;")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("syntax"));
}

#[test]
fn lint_csv_and_json() {
    stepq()
        .args(["lint", "-", "--format", "csv"])
        .write_stdin(LINT_ERRORS)
        .assert()
        .code(1)
        .stdout(predicate::str::diff(
            "severity,check,instance,message\n\
             error,shared-context,#30,representations #20 and #21 share context #10; a transformation needs two distinct contexts\n\
             error,dangling-reference,#40,references undefined instance #99\n\
             error,dangling-reference,#40,references undefined instance #99\n",
        ));

    let output = stepq()
        .args(["--format", "json", "lint", "-"])
        .write_stdin(LINT_ERRORS)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["file"], "<stdin>");
    assert_eq!(json["errors"], 3);
    assert_eq!(json["warnings"], 0);
    assert_eq!(json["file_schemas"], serde_json::json!(["DEMO"]));
    assert_eq!(json["checked_schema"], serde_json::Value::Null);
    assert_eq!(json["findings"][0]["check"], "shared-context");
    assert_eq!(json["findings"][0]["severity"], "error");
    assert_eq!(json["findings"][0]["instance"], 30);
}

#[test]
fn lint_reads_schemas_from_a_directory() {
    let dir = std::env::temp_dir().join(format!("stepq-lint-schemas-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("demo.exp"),
        "SCHEMA demo;
ENTITY application_context; application : STRING; END_ENTITY;
ENTITY application_protocol_definition;
  status : STRING; name : STRING; year : INTEGER; application : application_context;
END_ENTITY;
ENTITY point; name : STRING; END_ENTITY;
END_SCHEMA;",
    )
    .unwrap();
    let src = "ISO-10303-21;HEADER;FILE_SCHEMA(('DEMO'));ENDSEC;DATA;
#1=APPLICATION_PROTOCOL_DEFINITION('','demo',2026,#2);
#2=APPLICATION_CONTEXT('');
#3=POINT('p',1.);
ENDSEC;END-ISO-10303-21;";
    let assert = stepq()
        .args(["lint", "-", "--schema"])
        .arg(&dir)
        .write_stdin(src)
        .assert();
    std::fs::remove_dir_all(&dir).unwrap();
    assert
        .code(1)
        .stdout(predicate::str::starts_with(
            "<stdin>: checked against DEMO\n",
        ))
        .stdout(predicate::str::contains("error     attribute-count "))
        .stdout(predicate::str::contains(
            " #3  POINT: expected 1 attribute, found 2\n",
        ));

    stepq()
        .args(["lint", "-", "--schema", "no/such/schema.exp"])
        .write_stdin(src)
        .assert()
        .failure()
        .stderr(predicate::str::contains("reading no/such/schema.exp"));
}

#[test]
fn lint_table_lists_ten_findings_per_check_unless_all() {
    let products = (1..=12).fold(String::new(), |mut products, id| {
        writeln!(products, "#{id}=PRODUCT('p{id}','p{id}','',());").unwrap();
        products
    });
    let src = format!(
        "ISO-10303-21;HEADER;FILE_SCHEMA(('DEMO'));ENDSEC;DATA;
#100=APPLICATION_PROTOCOL_DEFINITION('','demo',2026,#101);
#101=APPLICATION_CONTEXT('');
{products}ENDSEC;END-ISO-10303-21;"
    );
    let limited = stepq()
        .args(["lint", "-"])
        .write_stdin(src.clone())
        .assert()
        .success();
    let stdout = String::from_utf8(limited.get_output().stdout.clone()).unwrap();
    assert_eq!(
        stdout.matches("uncategorized-product ").count(),
        11,
        "{stdout}"
    );
    assert!(stdout.contains("… 2 more (--all lists them)"), "{stdout}");
    assert!(stdout.ends_with("0 errors, 12 warnings\n"), "{stdout}");

    let all = stepq()
        .args(["lint", "-", "--all"])
        .write_stdin(src)
        .assert()
        .success();
    let stdout = String::from_utf8(all.get_output().stdout.clone()).unwrap();
    assert_eq!(
        stdout.matches("uncategorized-product ").count(),
        12,
        "{stdout}"
    );
    assert!(!stdout.contains("more (--all"), "{stdout}");
}

#[test]
fn refs_lists_both_directions() {
    stepq()
        .args(["refs", "-", "20"])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "#20=PRODUCT_DEFINITION('b-def','',#22,#3);\n\
             references\n  \
             #3=PRODUCT_DEFINITION_CONTEXT('part definition',#1,'design');\n  \
             #22=PRODUCT_DEFINITION_FORMATION_WITH_SPECIFIED_SOURCE('2','',#21,.MADE.);\n\
             referenced by\n  \
             #71=PRODUCT_DEFINITION_SHAPE('','',#20);\n  \
             #100=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u1','B1','',#10,#20,$);\n  \
             #101=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u2','B2','',#10,#20,$);\n  \
             #102=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u3','C1','',#20,#30,'X1');\n  \
             #103=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u4','C2','',#20,#30,'X2');\n  \
             #104=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u5','C3','',#20,#30,'X3');\n",
        ));
}

#[test]
fn refs_follows_references_to_a_depth_and_marks_repeats() {
    stepq()
        .args(["refs", "-", "#60", "--direction", "in", "--depth", "2"])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "#60=AXIS2_PLACEMENT_3D('',#61,$,$);\n\
             referenced by\n  \
             #50=SHAPE_REPRESENTATION('a',(#60),#1);\n    \
             #80=SHAPE_DEFINITION_REPRESENTATION(#70,#50);\n    \
             #91=(REPRESENTATION_RELATIONSHIP('','',#51,#50)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#92)…\n  \
             #51=SHAPE_REPRESENTATION('b',(#60),#1);\n    \
             #81=SHAPE_DEFINITION_REPRESENTATION(#71,#51);\n    \
             #91=(REPRESENTATION_RELATIONSHIP('','',#51,#50)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#92)… (*)\n    \
             #95=REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION('','',#51,#52,#92);\n  \
             #52=SHAPE_REPRESENTATION('c',(#60),#1);\n    \
             #82=SHAPE_DEFINITION_REPRESENTATION(#72,#52);\n    \
             #95=REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION('','',#51,#52,#92); (*)\n  \
             #92=ITEM_DEFINED_TRANSFORMATION('','',#60,#60);\n    \
             #91=(REPRESENTATION_RELATIONSHIP('','',#51,#50)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#92)… (*)\n    \
             #95=REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION('','',#51,#52,#92); (*)\n",
        ));
}

#[test]
fn refs_json_and_csv() {
    let output = stepq()
        .args(["--format", "json", "refs", "-", "70", "--full"])
        .write_stdin(ASSEMBLY)
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json[0]["id"], 70);
    assert_eq!(
        json[0]["types"],
        serde_json::json!(["PRODUCT_DEFINITION_SHAPE"])
    );
    assert_eq!(json[0]["references"][0]["id"], 10);
    assert_eq!(json[0]["referenced_by"][0]["id"], 80);
    assert_eq!(
        json[0]["referenced_by"][0]["text"],
        "#80=SHAPE_DEFINITION_REPRESENTATION(#70,#50);"
    );

    stepq()
        .args(["refs", "-", "70", "--direction", "out", "--format", "csv"])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "from,direction,level,instance,type,text\n\
             #70,references,1,#10,PRODUCT_DEFINITION,\"#10=PRODUCT_DEFINITION('a-def','',#12,#3);\"\n",
        ));
}

#[test]
fn refs_rejects_unknown_instances() {
    stepq()
        .args(["refs", "-", "999"])
        .write_stdin(ASSEMBLY)
        .assert()
        .failure()
        .stderr(predicate::str::contains("<stdin> has no instance #999"));
    stepq()
        .args(["refs", "-", "x1"])
        .write_stdin(ASSEMBLY)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not an instance name"));
}

#[test]
fn query_by_type_and_text() {
    stepq()
        .args(["query", "-", "--type", "product"])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "#11=PRODUCT('A','assembly','',(#2));\n\
             #21=PRODUCT('B','bracket assembly','',(#2));\n\
             #31=PRODUCT('C','bolt','',(#2));\n\
             #41=PRODUCT('D','plate','',(#2));\n\
             4 instances\n",
        ));
    stepq()
        .args(["query", "-", "--type", "PRODUCT", "--contains", "BOLT"])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "#31=PRODUCT('C','bolt','',(#2));\n1 instance\n",
        ));
    stepq()
        .args(["query", "-", "--type", "product", "--limit", "1"])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::ends_with(
            "#11=PRODUCT('A','assembly','',(#2));\n… 3 more (--limit)\n4 instances\n",
        ));
}

#[test]
fn query_matches_complex_instances_and_counts() {
    // #105 is complex; one of its partial entities is a NAUO.
    stepq()
        .args([
            "query",
            "-",
            "--type",
            "next_assembly_usage_occurrence",
            "--type",
            "quantified_assembly_component_usage",
            "--count",
        ])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "         6  NEXT_ASSEMBLY_USAGE_OCCURRENCE\n         \
             1  QUANTIFIED_ASSEMBLY_COMPONENT_USAGE\n",
        ));
    stepq()
        .args([
            "query",
            "-",
            "--type",
            "quantified_assembly_component_usage",
        ])
        .write_stdin(ASSEMBLY)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "#105=(ASSEMBLY_COMPONENT_USAGE($)NEXT_ASSEMBLY_USAGE_OCCURRENCE()PRODUCT_DEFINITION_RELATIONSHIP('u…\n\
             1 instance\n",
        ));

    let output = stepq()
        .args([
            "--format",
            "json",
            "query",
            "-",
            "--type",
            "measure_with_unit",
        ])
        .write_stdin(ASSEMBLY)
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json,
        serde_json::json!([{
            "id": 106,
            "types": ["MEASURE_WITH_UNIT"],
            "text": "#106=MEASURE_WITH_UNIT(COUNT_MEASURE(4.),#1);"
        }])
    );
}

/// Writes `contents` to a file in the temporary directory, unique to this
/// test process.
fn temp_file(name: &str, contents: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("stepq-cli-{}-{name}", std::process::id()));
    std::fs::write(&path, contents).unwrap();
    path
}

/// A header naming a person, a PERSON, and a product.
const PERSONAL: &str = "ISO-10303-21;HEADER;
FILE_NAME('bracket.stp','2026',('Jane Doe'),('ACME'),'','CAD 1','boss');
FILE_SCHEMA(('AP242'));ENDSEC;DATA;
#1=PERSON('jdoe','Doe','Jane',$,$,$);
#2=PRODUCT('BRK-100','bracket','secret',());
ENDSEC;END-ISO-10303-21;";

#[test]
fn strip_writes_to_standard_output_and_reports_on_standard_error() {
    stepq()
        .args(["strip", "-", "-o", "-"])
        .write_stdin(PERSONAL)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "FILE_NAME('bracket.stp','2026',(''),(''),'','CAD 1','');",
        ))
        .stdout(predicate::str::contains("#1=PERSON('','','',$,$,$);"))
        .stdout(predicate::str::contains(
            "#2=PRODUCT('BRK-100','bracket','secret',());",
        ))
        .stderr(predicate::str::diff(
            "<stdout>: replaced 6 strings in 1 instance\n",
        ));
}

#[test]
fn strip_anonymizes_into_a_file_and_will_not_overwrite_it() {
    let out = std::env::temp_dir().join(format!("stepq-cli-{}-strip.stp", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let first = stepq()
        .args(["strip", "-", "--anonymize", "-o"])
        .arg(&out)
        .write_stdin(PERSONAL)
        .assert();
    let written = std::fs::read_to_string(&out).unwrap_or_default();
    let again = stepq()
        .args(["strip", "-", "-o"])
        .arg(&out)
        .write_stdin(PERSONAL)
        .assert();
    let forced = stepq()
        .args(["strip", "-", "--force", "-o"])
        .arg(&out)
        .write_stdin(PERSONAL)
        .assert();
    let _ = std::fs::remove_file(&out);

    first.success().stdout(predicate::str::ends_with(
        "replaced 10 strings in 2 instances\n",
    ));
    assert!(
        written.contains("FILE_NAME('','2026',(''),(''),'','CAD 1','');"),
        "{written}"
    );
    assert!(
        written.contains("#2=PRODUCT('product-1','product-1','',());"),
        "{written}"
    );
    again
        .failure()
        .stderr(predicate::str::contains("already exists; pass --force"));
    forced.success();
}

#[test]
fn diff_of_identical_files_is_empty() {
    let copy = temp_file("diff-same.stp", ASSEMBLY);
    let assert = stepq()
        .args(["diff", "-"])
        .arg(&copy)
        .write_stdin(ASSEMBLY)
        .assert();
    std::fs::remove_file(&copy).unwrap();
    assert
        .success()
        .stdout(predicate::str::ends_with("\n\nno differences\n"));
}

#[test]
fn diff_reports_each_section_and_exits_one() {
    // C is renamed and one of B's three C usages is gone.
    let changed = ASSEMBLY
        .replace("PRODUCT('C','bolt'", "PRODUCT('C','hex bolt'")
        .replace(
            "#104=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u5','C3','',#20,#30,'X3');\n",
            "",
        );
    let new = temp_file("diff-new.stp", &changed);
    let table = stepq()
        .args(["diff", "-"])
        .arg(&new)
        .write_stdin(ASSEMBLY)
        .assert()
        .code(1);
    let stdout = String::from_utf8(table.get_output().stdout.clone()).unwrap();
    let only_products = stepq()
        .args(["diff", "-", "--section", "products"])
        .arg(&new)
        .write_stdin(ASSEMBLY)
        .assert()
        .code(1);
    let products = String::from_utf8(only_products.get_output().stdout.clone()).unwrap();
    let csv = stepq()
        .args(["diff", "-", "--section", "components", "--format", "csv"])
        .arg(&new)
        .write_stdin(ASSEMBLY)
        .assert()
        .code(1);
    let csv = String::from_utf8(csv.get_output().stdout.clone()).unwrap();
    std::fs::remove_file(&new).unwrap();

    assert!(stdout.starts_with("--- <stdin>\n+++ "), "{stdout}");
    assert!(
        stdout.contains("Header\n  instances: 40 → 39\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Entity types\n  NEXT_ASSEMBLY_USAGE_OCCURRENCE: 6 → 5 (-1)\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Products\n  ~ C: name: bolt → hex bolt\n"),
        "{stdout}"
    );
    assert!(stdout.contains("Components\n  B → C: 3 → 2\n"), "{stdout}");
    assert!(stdout.ends_with("\n4 differences\n"), "{stdout}");

    assert!(
        products.ends_with("Products\n  ~ C: name: bolt → hex bolt\n\n1 difference\n"),
        "{products}"
    );
    assert_eq!(
        csv,
        "section,change,subject,field,old,new\ncomponents,changed,B,C,3,2\n"
    );
}

#[test]
fn diff_takes_at_most_one_standard_input() {
    stepq()
        .args(["diff", "-", "-"])
        .write_stdin(ASSEMBLY)
        .assert()
        .failure()
        .stderr(predicate::str::contains("only one of the two files"));
}

/// A part with a validated volume on a shape aspect, a user-defined
/// attribute with three values, a property of nothing product-shaped, and a
/// persistent identifier.
const PROPERTIES: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('AP242'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('');
#10=PRODUCT('P1','bracket','',());
#11=PRODUCT_DEFINITION_FORMATION('','',#10);
#12=PRODUCT_DEFINITION('design','',#11,#13);
#13=PRODUCT_DEFINITION_CONTEXT('',#1,'design');
#20=PRODUCT_DEFINITION_SHAPE('','',#12);
#21=SHAPE_ASPECT('','solid',#20,.F.);
#30=PROPERTY_DEFINITION('geometric_validation_property','volume of solid',#21);
#31=PROPERTY_DEFINITION_REPRESENTATION(#30,#32);
#32=REPRESENTATION('volume',(#33),#1);
#33=MEASURE_REPRESENTATION_ITEM('volume measure',VOLUME_MEASURE(96858.91343205),#99);
#40=PROPERTY_DEFINITION('MATERIAL','user defined attribute',#12);
#41=GENERAL_PROPERTY('','MATERIAL','user defined attribute');
#42=GENERAL_PROPERTY_ASSOCIATION('user defined attribute','',#41,#40);
#43=PROPERTY_DEFINITION_REPRESENTATION(#40,#44);
#44=REPRESENTATION('',(#45,#46,#47),#1);
#45=DESCRIPTIVE_REPRESENTATION_ITEM('MATERIAL','Steel ''S235''');
#46=VALUE_REPRESENTATION_ITEM('THICKNESS',LENGTH_MEASURE(1.E0));
#47=(MEASURE_REPRESENTATION_ITEM()MEASURE_WITH_UNIT(MASS_MEASURE(2.5),#99)REPRESENTATION_ITEM('mass'));
#50=PROPERTY_DEFINITION('centroid','',#1);
#51=PROPERTY_DEFINITION_REPRESENTATION(#50,#52);
#52=REPRESENTATION('',(#53),#1);
#53=CARTESIAN_POINT('centre',(1.,2.,3.));
#70=ID_ATTRIBUTE('aspect.1',#21);
#99=DERIVED_UNIT(());
ENDSEC;END-ISO-10303-21;";

#[test]
fn props_table_groups_by_product_definition() {
    stepq()
        .args(["props", "-"])
        .write_stdin(PROPERTIES)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "bracket [P1]  #12\n  \
             validation  volume of solid / volume measure = 96858.91343205 (VOLUME_MEASURE, unit #99)  [on SHAPE_ASPECT #21]\n  \
             user        MATERIAL = Steel 'S235'\n  \
             user        MATERIAL / THICKNESS = 1.E0 (LENGTH_MEASURE)\n  \
             user        MATERIAL / mass = 2.5 (MASS_MEASURE, unit #99)\n  \
             id          aspect.1  [on SHAPE_ASPECT #21]\n\
             \n\
             (not attached to a product definition)\n  \
             other       centroid / centre = #53=CARTESIAN_POINT('centre',(1.,2.,3.));  [on APPLICATION_CONTEXT #1]\n\
             \n\
             3 properties, 5 values, 1 identifier\n",
        ));
}

#[test]
fn props_kind_filter() {
    stepq()
        .args(["props", "-", "--kind", "user", "--kind", "id"])
        .write_stdin(PROPERTIES)
        .assert()
        .success()
        .stdout(predicate::str::contains("validation").not())
        .stdout(predicate::str::contains("other ").not())
        .stdout(predicate::str::ends_with(
            "1 property, 3 values, 1 identifier\n",
        ));
}

#[test]
fn props_csv_and_json() {
    stepq()
        .args(["props", "-", "--kind", "validation", "--kind", "id", "--format", "csv"])
        .write_stdin(PROPERTIES)
        .assert()
        .success()
        .stdout(predicate::str::diff(
            "product_definition,product,kind,instance,name,description,subject,subject_type,value_instance,value_name,measure,value,unit\n\
             #12,bracket [P1],validation,#30,geometric_validation_property,volume of solid,#21,SHAPE_ASPECT,#33,volume measure,VOLUME_MEASURE,96858.91343205,#99\n\
             #12,bracket [P1],id,#70,aspect.1,,#21,SHAPE_ASPECT,,,,,\n",
        ));

    let output = stepq()
        .args(["--format", "json", "props", "-"])
        .write_stdin(PROPERTIES)
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["definitions"][0]["instance"], 12);
    let material = &json["properties"][1];
    assert_eq!(material["kind"], "user_defined");
    assert_eq!(material["subject"]["product_definition"], 12);
    assert_eq!(material["values"][0]["value"], "Steel 'S235'");
    assert_eq!(material["values"][1]["measure"], "LENGTH_MEASURE");
    assert_eq!(json["identifiers"][0]["id"], "aspect.1");
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
