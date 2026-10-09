//! Shared helpers for reading records: entity names, partial records,
//! attribute counts, numbers, representations and the containers that hold
//! topology.
//!
//! Every reader of stated values goes through these, so that "which record
//! of a complex instance holds this attribute", "how many attributes must it
//! have" and "which shell, solid or model holds what" are answered once.
//! `lint` keeps its own looser reader on purpose: it reports malformed
//! files, so it must not refuse what it is reporting on.

use std::borrow::Cow;

use crate::p21::{Exchange, Instance, Param, Record};

/// The upper-cased entity types of an instance; for a complex instance, its
/// partial entities joined with `+`, in the order written.
pub(crate) fn entity(exchange: &Exchange<'_>, instance: &Instance) -> String {
    exchange
        .records(instance)
        .map(|record| upper(record.name()))
        .collect::<Vec<_>>()
        .join("+")
}

/// An entity type name, upper-cased.
pub(crate) fn upper(name: &[u8]) -> String {
    String::from_utf8_lossy(name).to_ascii_uppercase()
}

/// The record named `name`, ignoring ASCII case: the record of a simple
/// instance of that type, or that partial entity of a complex instance.
pub(crate) fn partial<'e>(
    exchange: &'e Exchange<'_>,
    instance: &Instance,
    name: &str,
) -> Option<Record<'e>> {
    exchange.records(instance).find(|r| r.is(name))
}

/// The partial record named `name`, or failing that the first record.
pub(crate) fn primary_record<'e>(
    exchange: &'e Exchange<'_>,
    instance: &Instance,
    name: &str,
) -> Option<Record<'e>> {
    let mut records = exchange.records(instance);
    records
        .clone()
        .find(|r| r.is(name))
        .or_else(|| records.next())
}

/// The record's parameters if it has exactly `count`, `None` otherwise.
///
/// Readers of stated values check the attribute count of every record or
/// partial record they read: a record with another count does not have the
/// layout the reader assumes.
pub(crate) fn exact_params<'e>(record: Record<'e>, count: usize) -> Option<Vec<Param<'e>>> {
    let params: Vec<Param<'e>> = record.params().collect();
    (params.len() == count).then_some(params)
}

/// The referenced `#id`, if the parameter is a reference.
pub(crate) fn reference(param: Option<&Param<'_>>) -> Option<u64> {
    param.and_then(Param::reference)
}

/// An INTEGER or REAL token as a finite number. `None` for anything else,
/// including a typed value and a number too large for an `f64`.
pub(crate) fn real(param: &Param<'_>) -> Option<f64> {
    match param {
        Param::Integer(literal) | Param::Real(literal) => {
            literal.to_f64().filter(|value| value.is_finite())
        }
        _ => None,
    }
}

/// A string attribute, decoded; `None` if unset or not a string.
pub(crate) fn text(param: Option<Param<'_>>) -> Option<String> {
    let literal = param?.literal()?;
    if literal.text().first() != Some(&b'\'') {
        return None;
    }
    literal.decode().ok().map(Cow::into_owned)
}

/// What `representation` reads: `representation.items` and
/// `representation.context_of_items`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Representation {
    /// The `#id`s of the items, in order.
    pub(crate) items: Vec<u64>,
    /// The `#id` of the context, which is not checked here.
    pub(crate) context: u64,
}

/// Why [`representation`] could not read an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotRepresentation {
    /// No instance has this `#id`.
    Undefined,
    /// The instance does not have the layout of a representation.
    Layout,
}

/// Reads instance `#id` as a representation: `(name, items, context)`.
///
/// A simple instance is read from its first three attributes, which every
/// subtype of `representation` inherits in that order; a subtype may add
/// attributes after them (`tessellated_shape_representation_with_accuracy_parameters`
/// adds a fourth). A complex instance is read from its `REPRESENTATION`
/// partial, which must have exactly three. Either way `name` must be a
/// string, `items` a list of references and `context_of_items` a reference.
pub(crate) fn representation(
    exchange: &Exchange<'_>,
    id: u64,
) -> Result<Representation, NotRepresentation> {
    let instance = exchange.get(id).ok_or(NotRepresentation::Undefined)?;
    let params: Vec<Param<'_>> = if instance.is_complex() {
        partial(exchange, instance, "REPRESENTATION")
            .and_then(|record| exact_params(record, 3))
            .ok_or(NotRepresentation::Layout)?
    } else {
        let record = exchange
            .records(instance)
            .next()
            .ok_or(NotRepresentation::Layout)?;
        let params: Vec<Param<'_>> = record.params().take(3).collect();
        if params.len() < 3 {
            return Err(NotRepresentation::Layout);
        }
        params
    };
    let [name, items, context] = params.as_slice() else {
        return Err(NotRepresentation::Layout);
    };
    if !matches!(name, Param::String(_)) {
        return Err(NotRepresentation::Layout);
    }
    let items = items
        .list()
        .ok_or(NotRepresentation::Layout)?
        .map(|item| item.reference())
        .collect::<Option<Vec<u64>>>()
        .ok_or(NotRepresentation::Layout)?;
    let context = context.reference().ok_or(NotRepresentation::Layout)?;
    Ok(Representation { items, context })
}

/// True if the record is a representation context or one of its subtypes,
/// such as `GEOMETRIC_REPRESENTATION_CONTEXT` or
/// `GLOBAL_UNIT_ASSIGNED_CONTEXT`.
pub(crate) fn is_context_record(record: Record<'_>) -> bool {
    let name = upper(record.name());
    name.ends_with("REPRESENTATION_CONTEXT") || name.ends_with("ASSIGNED_CONTEXT")
}

/// A topological container: an entity that holds faces, shells or other
/// containers in some of its attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Container {
    /// The entity type, upper case.
    pub(crate) entity: &'static str,
    /// Its exact attribute count, as a simple instance.
    pub(crate) params: usize,
    /// The attributes that hold its parts, by position: a reference or a
    /// list of references.
    pub(crate) holds: &'static [usize],
}

/// Which attribute of which shell, solid or model holds what (ISO
/// 10303-42). Units and topology both walk containment through this table,
/// so the two cannot disagree about what holds a face.
///
/// Only simple instances are containers. A complex instance of one of these
/// types, or any container not listed, is not followed: a walk that needs it
/// reports what it could not reach instead of guessing.
pub(crate) const CONTAINERS: &[Container] = &[
    // connected_face_set.cfs_faces
    Container {
        entity: "CLOSED_SHELL",
        params: 2,
        holds: &[1],
    },
    Container {
        entity: "OPEN_SHELL",
        params: 2,
        holds: &[1],
    },
    Container {
        entity: "CONNECTED_FACE_SET",
        params: 2,
        holds: &[1],
    },
    // oriented_closed_shell(name, *, closed_shell_element, orientation):
    // cfs_faces is derived and written `*`.
    Container {
        entity: "ORIENTED_CLOSED_SHELL",
        params: 4,
        holds: &[2],
    },
    Container {
        entity: "ORIENTED_OPEN_SHELL",
        params: 4,
        holds: &[2],
    },
    // manifold_solid_brep.outer
    Container {
        entity: "MANIFOLD_SOLID_BREP",
        params: 2,
        holds: &[1],
    },
    Container {
        entity: "FACETED_BREP",
        params: 2,
        holds: &[1],
    },
    // brep_with_voids(name, outer, voids)
    Container {
        entity: "BREP_WITH_VOIDS",
        params: 3,
        holds: &[1, 2],
    },
    // shell_based_surface_model.sbsm_boundary
    Container {
        entity: "SHELL_BASED_SURFACE_MODEL",
        params: 2,
        holds: &[1],
    },
    // face_based_surface_model.fbsm_faces
    Container {
        entity: "FACE_BASED_SURFACE_MODEL",
        params: 2,
        holds: &[1],
    },
];

/// The container entry for a record, if it is a simple instance of a
/// container type.
pub(crate) fn container(record: Record<'_>) -> Option<&'static Container> {
    CONTAINERS
        .iter()
        .find(|container| record.is(container.entity))
}

/// True if `param` is `#id` or a list that contains `#id`.
pub(crate) fn holds(param: &Param<'_>, id: u64) -> bool {
    match param {
        Param::Reference(target) => *target == id,
        Param::List(items) => items.clone().any(|item| item.reference() == Some(id)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::parse;

    fn exchange(data: &str) -> Exchange<'static> {
        let src = format!(
            "ISO-10303-21;HEADER;FILE_SCHEMA(('X'));ENDSEC;DATA;{data}ENDSEC;END-ISO-10303-21;"
        );
        let src: &'static [u8] = Box::leak(src.into_bytes().into_boxed_slice());
        parse(src).unwrap()
    }

    #[test]
    fn entities_and_partials() {
        let x = exchange(
            "#1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));#2=product('a','b','',());",
        );
        let unit = x.get(1).unwrap();
        assert_eq!(entity(&x, unit), "LENGTH_UNIT+NAMED_UNIT+SI_UNIT");
        assert_eq!(entity(&x, x.get(2).unwrap()), "PRODUCT");
        let si = partial(&x, unit, "si_unit").unwrap();
        assert_eq!(si.name(), b"SI_UNIT");
        assert!(partial(&x, unit, "MASS_UNIT").is_none());
        assert_eq!(
            primary_record(&x, unit, "MASS_UNIT").unwrap().name(),
            b"LENGTH_UNIT",
            "falls back to the first record"
        );
        assert_eq!(exact_params(si, 2).map(|p| p.len()), Some(2));
        assert!(exact_params(si, 3).is_none());
    }

    #[test]
    fn numbers_and_strings() {
        let x = exchange("#1=A(1,2.5,1.E999,'x',$,LENGTH_MEASURE(3.),'it''s');");
        let record = x.records(x.get(1).unwrap()).next().unwrap();
        let params: Vec<Param<'_>> = record.params().collect();
        assert_eq!(real(&params[0]), Some(1.0), "an INTEGER is a number");
        assert_eq!(real(&params[1]), Some(2.5));
        assert_eq!(real(&params[2]), None, "not finite");
        assert_eq!(real(&params[3]), None);
        assert_eq!(real(&params[4]), None);
        assert_eq!(real(&params[5]), None, "a typed value is not a bare number");
        assert_eq!(text(record.param(6)).as_deref(), Some("it's"));
        assert_eq!(text(record.param(4)), None);
        assert_eq!(reference(record.param(0).as_ref()), None);
    }

    #[test]
    fn representations_simple_complex_and_subtypes() {
        let x = exchange(
            "#1=SHAPE_REPRESENTATION('',(#5,#6),#9);\
             #2=(REPRESENTATION('',(#5),#9)SHAPE_REPRESENTATION());\
             #3=TESSELLATED_SHAPE_REPRESENTATION_WITH_ACCURACY_PARAMETERS('',(#5),#9,(0.1));\
             #4=SHAPE_DEFINITION_REPRESENTATION(#7,#1);\
             #8=(REPRESENTATION('',(#5),#9,#9)SHAPE_REPRESENTATION());\
             #10=SHAPE_REPRESENTATION('',(#5,'x'),#9);\
             #11=SHAPE_REPRESENTATION($,(#5),#9);",
        );
        assert_eq!(
            representation(&x, 1),
            Ok(Representation {
                items: vec![5, 6],
                context: 9
            })
        );
        assert_eq!(representation(&x, 2).map(|r| r.items), Ok(vec![5]));
        assert_eq!(
            representation(&x, 3).map(|r| r.context),
            Ok(9),
            "a subtype's own attributes come after the inherited three"
        );
        assert_eq!(representation(&x, 4), Err(NotRepresentation::Layout));
        assert_eq!(
            representation(&x, 8),
            Err(NotRepresentation::Layout),
            "a REPRESENTATION partial has exactly three attributes"
        );
        assert_eq!(representation(&x, 10), Err(NotRepresentation::Layout));
        assert_eq!(representation(&x, 11), Err(NotRepresentation::Layout));
        assert_eq!(representation(&x, 99), Err(NotRepresentation::Undefined));
    }

    #[test]
    fn containers() {
        let x = exchange(
            "#1=CLOSED_SHELL('',(#5,#6));#2=ORIENTED_CLOSED_SHELL('',*,#1,.F.);\
             #3=BREP_WITH_VOIDS('',#1,(#2));#4=ADVANCED_FACE('',(),#9,.T.);",
        );
        let record = |id| x.records(x.get(id).unwrap()).next().unwrap();
        let shell = container(record(1)).unwrap();
        assert!(holds(&record(1).param(shell.holds[0]).unwrap(), 6));
        assert_eq!(container(record(2)).unwrap().holds, &[2]);
        let voids = container(record(3)).unwrap();
        assert!(
            voids
                .holds
                .iter()
                .any(|&i| holds(&record(3).param(i).unwrap(), 2))
        );
        assert!(container(record(4)).is_none());
    }

    #[test]
    fn context_records() {
        let x = exchange(
            "#1=GLOBAL_UNIT_ASSIGNED_CONTEXT('','',(#9));\
             #2=(GEOMETRIC_REPRESENTATION_CONTEXT(3)REPRESENTATION_CONTEXT('',''));\
             #3=SHAPE_REPRESENTATION('',(),#2);",
        );
        let any_context = |id| x.records(x.get(id).unwrap()).any(is_context_record);
        assert!(any_context(1));
        assert!(any_context(2));
        assert!(!any_context(3));
    }
}
