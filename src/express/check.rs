//! Checks a parsed file's records against a schema's attribute layout.

use std::collections::HashMap;
use std::fmt;

use super::schema::{Schema, Slot};
use crate::p21::{Exchange, Record};

/// Record layouts already computed, keyed by whether the instance is
/// complex and its upper-cased record names; `None` if unresolvable.
type Layouts = HashMap<(bool, Vec<String>), Option<Vec<Vec<Slot>>>>;

/// One way an instance does not fit the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Problem {
    /// The instance name, `#id`.
    pub instance: u64,
    /// The upper-cased entity type of the record at fault.
    pub entity: String,
    /// What is wrong.
    pub kind: ProblemKind,
}

/// What is wrong with a record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProblemKind {
    /// The entity type, or one of its supertypes, is not in the schema.
    UnknownEntity,
    /// The record has the wrong number of attributes.
    AttributeCount {
        /// Attributes the schema declares for this record.
        expected: usize,
        /// Attributes the record has.
        found: usize,
    },
    /// A list has fewer elements than its aggregate type's lower bound.
    TooFewElements {
        /// The attribute.
        attribute: String,
        /// The lower bound.
        min: u64,
        /// Elements present.
        found: usize,
    },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            instance, entity, ..
        } = self;
        match &self.kind {
            ProblemKind::UnknownEntity => {
                write!(f, "#{instance} {entity}: entity type is not in the schema")
            }
            ProblemKind::AttributeCount { expected, found } => write!(
                f,
                "#{instance} {entity}: expected {expected} attribute{}, found {found}",
                if *expected == 1 { "" } else { "s" }
            ),
            ProblemKind::TooFewElements {
                attribute,
                min,
                found,
            } => write!(
                f,
                "#{instance} {entity}.{attribute}: needs at least {min} element{}, found {found}",
                if *min == 1 { "" } else { "s" }
            ),
        }
    }
}

/// Checks every record in `exchange` against `schema`: that its entity
/// type exists, that it has as many attributes as the schema declares, and
/// that no list is shorter than its aggregate type allows. Problems are
/// returned in file order.
///
/// Simple instances are checked against the full inherited layout
/// ([`Schema::slots`]); each partial record of a complex instance against
/// its own attributes ([`Schema::complex_slots`]).
pub fn check(schema: &Schema, exchange: &Exchange<'_>) -> Vec<Problem> {
    let mut layouts: Layouts = HashMap::new();
    let mut problems = Vec::new();
    for instance in exchange.instances() {
        let records: Vec<Record<'_>> = exchange.records(instance).collect();
        let names: Vec<String> = records
            .iter()
            .map(|record| String::from_utf8_lossy(record.name()).to_ascii_uppercase())
            .collect();
        let complex = instance.is_complex();
        let layout = layouts.entry((complex, names.clone())).or_insert_with(|| {
            let partials: Vec<&str> = names.iter().map(String::as_str).collect();
            if complex {
                schema.complex_slots(&partials)
            } else {
                partials
                    .first()
                    .and_then(|name| schema.slots(name))
                    .map(|slots| vec![slots])
            }
        });

        let Some(layout) = layout else {
            let entity = names
                .iter()
                .find(|name| schema.slots(name).is_none())
                .or(names.first())
                .cloned()
                .unwrap_or_default();
            problems.push(Problem {
                instance: instance.id,
                entity,
                kind: ProblemKind::UnknownEntity,
            });
            continue;
        };

        for ((record, slots), name) in records.iter().zip(layout.iter()).zip(&names) {
            let params: Vec<_> = record.params().collect();
            if params.len() != slots.len() {
                problems.push(Problem {
                    instance: instance.id,
                    entity: name.clone(),
                    kind: ProblemKind::AttributeCount {
                        expected: slots.len(),
                        found: params.len(),
                    },
                });
                continue;
            }
            for (param, slot) in params.iter().zip(slots) {
                let (Some(min), Some(items)) = (slot.min_len, param.list()) else {
                    continue;
                };
                let found = items.count();
                if u64::try_from(found).unwrap_or(u64::MAX) < min {
                    problems.push(Problem {
                        instance: instance.id,
                        entity: name.clone(),
                        kind: ProblemKind::TooFewElements {
                            attribute: slot.name.clone(),
                            min,
                            found,
                        },
                    });
                }
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::parse;

    const SCHEMA: &str = "
SCHEMA demo;
TYPE label = STRING; END_TYPE;
TYPE nonempty = SET [1:?] OF label; END_TYPE;
ENTITY named_unit; dimensions : label; END_ENTITY;
ENTITY si_unit SUBTYPE OF (named_unit);
  prefix : OPTIONAL label;
  name : label;
DERIVE
  SELF\\named_unit.dimensions : label := 'x';
END_ENTITY;
ENTITY point; name : label; coordinates : LIST [1:3] OF REAL; END_ENTITY;
ENTITY line SUBTYPE OF (point); tags : nonempty; END_ENTITY;
END_SCHEMA;
";

    #[test]
    fn problems_are_reported_in_file_order() {
        let schema = Schema::parse(SCHEMA.as_bytes()).unwrap();
        let src = "ISO-10303-21;HEADER;FILE_SCHEMA(('DEMO'));ENDSEC;DATA;
#1=POINT('p',(0.,0.,0.));
#2=POINT('short');
#3=(NAMED_UNIT(*)SI_UNIT($,'metre'));
#4=LINE('l',(1.),());
#5=MYSTERY();
#6=(NAMED_UNIT(*)SI_UNIT($));
#7=(NAMED_UNIT(*)MYSTERY());
#8=point('lower case',(2.));
ENDSEC;END-ISO-10303-21;";
        let exchange = parse(src.as_bytes()).unwrap();
        let problems: Vec<String> = check(&schema, &exchange)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            problems,
            [
                "#2 POINT: expected 2 attributes, found 1",
                "#4 LINE.TAGS: needs at least 1 element, found 0",
                "#5 MYSTERY: entity type is not in the schema",
                "#6 SI_UNIT: expected 2 attributes, found 1",
                "#7 MYSTERY: entity type is not in the schema",
            ]
        );
    }
}
