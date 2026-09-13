//! Removing personal and identifying text from a STEP file.
//!
//! This is what `stepq strip` writes. Only string literals change, each
//! replaced where it stands: geometry, numbers, instance names and entity
//! types are copied byte for byte by [`Writer`](crate::p21::Writer) with the
//! [`Replacements`] this module plans. Strings that carry meaning for a
//! reader — contexts, categories, colour and font names, the names of
//! validation properties and user attributes — are left alone.

use std::collections::HashSet;

use crate::p21::{Exchange, Param, Record, Replacements};

/// What [`strip`] plans to change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Stripped {
    /// The strings to write instead, for [`Writer::replacements`](crate::p21::Writer::replacements).
    pub replacements: Replacements,
    /// How many instances have a replaced string; the header is not
    /// counted.
    pub instances: usize,
}

/// Plans the replacements for `exchange`.
///
/// Always blanks the header's `FILE_NAME` author, organization and
/// authorization, and every string of `PERSON`, `ORGANIZATION` and address
/// instances. With `anonymize`, also renames every `PRODUCT` to
/// `product-1`, `product-2`, … (both id and name) and every assembly usage
/// id to `usage-1`, …, and blanks: the header's file name; product,
/// formation and definition descriptions; usage names, descriptions and
/// reference designators; shape representation and product definition
/// shape names; and the text of descriptive representation items, which is
/// where user-defined attribute values live.
pub fn strip(exchange: &Exchange<'_>, anonymize: bool) -> Stripped {
    let mut plan = Plan::default();

    if let Some(file_name) = exchange.header_entity("FILE_NAME") {
        if anonymize {
            plan.blank(file_name.param(0));
        }
        plan.blank_all(file_name.param(2));
        plan.blank_all(file_name.param(3));
        plan.blank(file_name.param(6));
    }

    let mut products = 0;
    let mut usages = 0;
    let mut touched = HashSet::new();
    for instance in exchange.instances() {
        let before = plan.replacements.len();
        let records: Vec<Record<'_>> = exchange.records(instance).collect();
        let personal = records.iter().any(|record| {
            let name = upper(record.name());
            name == "PERSON" || name == "ORGANIZATION" || name.ends_with("ADDRESS")
        });
        if personal {
            for record in &records {
                for param in record.params() {
                    plan.blank_all(Some(param));
                }
            }
        } else if anonymize {
            for record in &records {
                match upper(record.name()).as_str() {
                    "PRODUCT" => {
                        products += 1;
                        let label = format!("'product-{products}'");
                        plan.set(record.param(0), &label);
                        plan.set(record.param(1), &label);
                        plan.blank(record.param(2));
                    }
                    "PRODUCT_DEFINITION"
                    | "PRODUCT_DEFINITION_FORMATION"
                    | "PRODUCT_DEFINITION_FORMATION_WITH_SPECIFIED_SOURCE" => {
                        plan.blank(record.param(1));
                    }
                    "NEXT_ASSEMBLY_USAGE_OCCURRENCE" if !instance.is_complex() => {
                        usages += 1;
                        plan.set(record.param(0), &format!("'usage-{usages}'"));
                        plan.blank(record.param(1));
                        plan.blank(record.param(2));
                        plan.blank(record.param(5));
                    }
                    // The partial entities of a complex usage.
                    "PRODUCT_DEFINITION_RELATIONSHIP" => {
                        usages += 1;
                        plan.set(record.param(0), &format!("'usage-{usages}'"));
                        plan.blank(record.param(1));
                        plan.blank(record.param(2));
                    }
                    "ASSEMBLY_COMPONENT_USAGE" => plan.blank(record.param(0)),
                    "PRODUCT_DEFINITION_SHAPE" => {
                        plan.blank(record.param(0));
                        plan.blank(record.param(1));
                    }
                    "DESCRIPTIVE_REPRESENTATION_ITEM" => plan.blank(record.param(1)),
                    name if name.ends_with("SHAPE_REPRESENTATION") => {
                        plan.blank(record.param(0));
                    }
                    _ => {}
                }
            }
        }
        if plan.replacements.len() > before {
            touched.insert(instance.id);
        }
    }

    Stripped {
        replacements: plan.replacements,
        instances: touched.len(),
    }
}

fn upper(name: &[u8]) -> String {
    String::from_utf8_lossy(name).to_ascii_uppercase()
}

#[derive(Default)]
struct Plan {
    replacements: Replacements,
}

impl Plan {
    /// Replaces a string attribute with `text`, if it is a string that
    /// differs.
    fn set(&mut self, param: Option<Param<'_>>, text: &str) {
        let Some(literal) = param.and_then(|param| param.literal()) else {
            return;
        };
        let source = literal.text();
        if source.first() == Some(&b'\'') && source != text.as_bytes() {
            self.replacements.replace(literal.span(), text);
        }
    }

    /// Blanks a string attribute.
    fn blank(&mut self, param: Option<Param<'_>>) {
        self.set(param, "''");
    }

    /// Blanks a string attribute, or every string in a list, at any depth.
    fn blank_all(&mut self, param: Option<Param<'_>>) {
        let Some(param) = param else {
            return;
        };
        match param.list() {
            Some(items) => {
                for item in items {
                    self.blank_all(Some(item));
                }
            }
            None => self.blank(Some(param)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::{Writer, parse};

    const SAMPLE: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('bracket'),'2;1');
FILE_NAME('bracket.stp','2026-01-01',('Jane Doe'),('ACME Corp'),'pre 1','CAD 1','the boss');
FILE_SCHEMA(('AP242'));
ENDSEC;
DATA;
#1=APPLICATION_CONTEXT('mechanical design');
#2=PRODUCT_CONTEXT('',#1,'mechanical');
#3=PRODUCT_DEFINITION_CONTEXT('part definition',#1,'design');
#4=PERSON('jdoe','Doe','Jane',('Q.'),$,$);
#5=ORGANIZATION('acme','ACME Corp','makes brackets');
#6=PERSONAL_ADDRESS('Springfield',$,$,$,$,$,$,$,$,$,$,$,(#4),'home');
#10=PRODUCT('BRK-100','bracket','top secret',(#2));
#11=PRODUCT_DEFINITION_FORMATION('A','first release',#10);
#12=PRODUCT_DEFINITION('design','for customer X',#11,#3);
#13=NEXT_ASSEMBLY_USAGE_OCCURRENCE('NAUO7','BOLT:1','',#12,#12,'R1');
#14=PRODUCT_DEFINITION_SHAPE('bracket shape','',#12);
#15=SHAPE_REPRESENTATION('bracket body',(),#1);
#16=DESCRIPTIVE_REPRESENTATION_ITEM('MATERIAL','Unobtainium');
#17=PROPERTY_DEFINITION('geometric_validation_property','volume',#14);
#18=CARTESIAN_POINT('',(1.50,2.,3.));
ENDSEC;
END-ISO-10303-21;
";

    fn stripped(anonymize: bool) -> (String, Stripped) {
        let exchange = parse(SAMPLE.as_bytes()).unwrap();
        let plan = strip(&exchange, anonymize);
        let mut out = Vec::new();
        Writer::new(&exchange)
            .replacements(&plan.replacements)
            .write_all(&mut out)
            .unwrap();
        (String::from_utf8(out).unwrap(), plan)
    }

    #[test]
    fn personal_data_is_always_blanked() {
        let (text, plan) = stripped(false);
        assert!(
            text.contains("FILE_NAME('bracket.stp','2026-01-01',(''),(''),'pre 1','CAD 1','');"),
            "{text}"
        );
        assert!(text.contains("#4=PERSON('','','',(''),$,$);"), "{text}");
        assert!(text.contains("#5=ORGANIZATION('','','');"), "{text}");
        assert!(
            text.contains("#6=PERSONAL_ADDRESS('',$,$,$,$,$,$,$,$,$,$,$,(#4),'');"),
            "{text}"
        );
        // Products are untouched without --anonymize.
        assert!(text.contains("#10=PRODUCT('BRK-100','bracket','top secret',(#2));"));
        assert_eq!(plan.instances, 3);
    }

    #[test]
    fn anonymize_renames_products_and_blanks_descriptions() {
        let (text, plan) = stripped(true);
        for expected in [
            "FILE_NAME('','2026-01-01',(''),(''),'pre 1','CAD 1','');",
            "#10=PRODUCT('product-1','product-1','',(#2));",
            "#11=PRODUCT_DEFINITION_FORMATION('A','',#10);",
            "#12=PRODUCT_DEFINITION('design','',#11,#3);",
            "#13=NEXT_ASSEMBLY_USAGE_OCCURRENCE('usage-1','','',#12,#12,'');",
            "#14=PRODUCT_DEFINITION_SHAPE('','',#12);",
            "#15=SHAPE_REPRESENTATION('',(),#1);",
            "#16=DESCRIPTIVE_REPRESENTATION_ITEM('MATERIAL','');",
            // Strings with meaning, and everything that is not a string, stay.
            "#1=APPLICATION_CONTEXT('mechanical design');",
            "#3=PRODUCT_DEFINITION_CONTEXT('part definition',#1,'design');",
            "#17=PROPERTY_DEFINITION('geometric_validation_property','volume',#14);",
            "#18=CARTESIAN_POINT('',(1.50,2.,3.));",
            "FILE_DESCRIPTION(('bracket'),'2;1');",
        ] {
            assert!(text.contains(expected), "missing {expected}\n{text}");
        }
        assert_eq!(plan.instances, 10);
        assert_eq!(parse(text.as_bytes()).unwrap().instances().len(), 15);
    }
}
