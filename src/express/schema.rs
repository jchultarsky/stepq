//! The parsed schema and the attribute layouts it implies.

use std::collections::HashMap;

use crate::error::Result;

/// Inheritance chains deeper than this are treated as unresolvable, so a
/// malformed schema cannot exhaust the stack.
const MAX_INHERITANCE_DEPTH: usize = 256;

/// A parsed EXPRESS schema; see the [module documentation](super).
#[derive(Debug, Clone)]
pub struct Schema {
    name: String,
    entities: Vec<Entity>,
    index: HashMap<String, usize>,
    types: HashMap<String, TypeDef>,
}

/// An entity declaration. Names are upper-cased.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Entity {
    /// The entity name.
    pub name: String,
    /// True for `ABSTRACT SUPERTYPE`.
    pub is_abstract: bool,
    /// Direct supertypes, in `SUBTYPE OF` order.
    pub supertypes: Vec<String>,
    /// Explicit attributes declared by this entity, in order. These are the
    /// attributes the entity adds to a record.
    pub attributes: Vec<Attribute>,
    /// Inherited explicit attributes this entity redeclares with a narrower
    /// type. A redeclaration adds no attribute to a record.
    pub redeclared: Vec<Redeclaration>,
    /// Inherited explicit attributes this entity redeclares as derived.
    /// Their value is written `*` in a Part 21 record.
    pub derived: Vec<AttributeRef>,
}

/// An explicit attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Attribute {
    /// The attribute name.
    pub name: String,
    /// True for `OPTIONAL`: the value may be `$`.
    pub optional: bool,
    /// The declared type.
    pub ty: TypeRef,
}

/// `SELF\entity.attribute` in a redeclaration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AttributeRef {
    /// The supertype named after `SELF\`.
    pub entity: String,
    /// The attribute name.
    pub attribute: String,
}

/// An explicit redeclaration of an inherited attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Redeclaration {
    /// The attribute being redeclared.
    pub attribute: AttributeRef,
    /// True if the redeclaration is `OPTIONAL`.
    pub optional: bool,
    /// The narrowed type.
    pub ty: TypeRef,
}

/// A type as written in an attribute or type declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TypeRef {
    /// A defined type or entity, by upper-cased name.
    Named(String),
    /// A built-in simple type such as `STRING`, `REAL` or `LOGICAL`.
    Builtin(String),
    /// `SET`, `BAG`, `LIST`, `ARRAY` or `AGGREGATE`.
    Aggregate {
        /// Which aggregate.
        kind: AggregateKind,
        /// The lower bound when it is a literal; `SET OF x` without bounds
        /// has lower bound 0. `None` for a bound given as an expression.
        lower: Option<u64>,
        /// The upper bound when it is a literal; `None` for `?` or an
        /// expression.
        upper: Option<u64>,
        /// The element type.
        element: Box<TypeRef>,
    },
}

/// The kind of an aggregate type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AggregateKind {
    /// `ARRAY`
    Array,
    /// `BAG`
    Bag,
    /// `LIST`
    List,
    /// `SET`
    Set,
    /// `AGGREGATE`, in algorithm parameters.
    Aggregate,
}

/// The underlying type of a `TYPE` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TypeDef {
    /// `TYPE t = <type>;`
    Alias(TypeRef),
    /// A `SELECT` or `ENUMERATION`, whose contents do not affect layout.
    Constructed,
}

/// One attribute position in a Part 21 record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Slot {
    /// The entity that declares the attribute.
    pub entity: String,
    /// The attribute name.
    pub name: String,
    /// True if the value may be `$`.
    pub optional: bool,
    /// True if a subtype redeclares the attribute as derived, so the value
    /// is written `*`.
    pub derived: bool,
    /// For an aggregate attribute with a literal lower bound, the minimum
    /// number of elements, taking redeclarations and defined types into
    /// account. `None` otherwise.
    pub min_len: Option<u64>,
}

#[derive(Default)]
struct Override {
    derived: bool,
    ty: Option<TypeRef>,
}

impl Schema {
    pub(crate) fn new(
        name: String,
        entities: Vec<Entity>,
        types: HashMap<String, TypeDef>,
    ) -> Self {
        let index = entities
            .iter()
            .enumerate()
            .map(|(i, entity)| (entity.name.clone(), i))
            .collect();
        Self {
            name,
            entities,
            index,
            types,
        }
    }

    /// Parses the first schema in an EXPRESS source file.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Syntax`](crate::Error::Syntax) if the parts of the
    /// schema this reader interprets are malformed.
    pub fn parse(src: &[u8]) -> Result<Self> {
        super::parser::parse(src)
    }

    /// The upper-cased schema name, such as `AUTOMOTIVE_DESIGN`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// True if a `FILE_SCHEMA` identifier names this schema. The object
    /// identifier in braces, if any, is ignored.
    pub fn matches(&self, file_schema: &str) -> bool {
        file_schema
            .split(|c: char| c.is_whitespace() || c == '{')
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case(&self.name))
    }

    /// Every entity, in declaration order.
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// The entity named `name`, ignoring ASCII case.
    pub fn entity(&self, name: &str) -> Option<&Entity> {
        self.position(name).map(|i| &self.entities[i])
    }

    /// The defined type named `name`, ignoring ASCII case.
    pub fn type_def(&self, name: &str) -> Option<&TypeDef> {
        self.types.get(&name.to_ascii_uppercase())
    }

    /// The attribute layout of a simple instance of `entity`: the explicit
    /// attributes of every supertype, depth-first in `SUBTYPE OF` order and
    /// each supertype once, followed by the entity's own.
    ///
    /// `None` if the entity, or one of its supertypes, is not in the schema.
    pub fn slots(&self, entity: &str) -> Option<Vec<Slot>> {
        let order = self.closure(&[entity])?;
        let overrides = self.overrides(&order);
        Some(
            order
                .iter()
                .flat_map(|&i| self.own_slots(i, &overrides))
                .collect(),
        )
    }

    /// The attribute layout of each partial record of a complex instance
    /// made of `partials`, in the order given. Each partial record holds only
    /// the attributes its own entity declares; whether one is derived
    /// depends on all the partials together.
    ///
    /// `None` if any partial entity, or a supertype, is not in the schema.
    pub fn complex_slots(&self, partials: &[&str]) -> Option<Vec<Vec<Slot>>> {
        let order = self.closure(partials)?;
        let overrides = self.overrides(&order);
        partials
            .iter()
            .map(|name| self.position(name).map(|i| self.own_slots(i, &overrides)))
            .collect()
    }

    /// The minimum number of elements of `ty` when it is, or is defined as,
    /// an aggregate with a literal lower bound.
    pub fn min_len(&self, ty: &TypeRef) -> Option<u64> {
        let mut ty = ty;
        for _ in 0..MAX_INHERITANCE_DEPTH {
            match ty {
                TypeRef::Aggregate { lower, .. } => return *lower,
                TypeRef::Named(name) => match self.types.get(name)? {
                    TypeDef::Alias(underlying) => ty = underlying,
                    TypeDef::Constructed => return None,
                },
                TypeRef::Builtin(_) => return None,
            }
        }
        None
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.index.get(&name.to_ascii_uppercase()).copied()
    }

    /// `roots` and all their supertypes, supertypes before subtypes, each
    /// once.
    fn closure(&self, roots: &[&str]) -> Option<Vec<usize>> {
        let mut order = Vec::new();
        let mut seen = vec![false; self.entities.len()];
        for root in roots {
            self.visit(self.position(root)?, &mut seen, &mut order, 0)?;
        }
        Some(order)
    }

    fn visit(
        &self,
        entity: usize,
        seen: &mut [bool],
        order: &mut Vec<usize>,
        depth: usize,
    ) -> Option<()> {
        if seen[entity] {
            return Some(());
        }
        if depth > MAX_INHERITANCE_DEPTH {
            return None;
        }
        seen[entity] = true;
        for supertype in &self.entities[entity].supertypes {
            self.visit(self.position(supertype)?, seen, order, depth + 1)?;
        }
        order.push(entity);
        Some(())
    }

    /// Redeclarations made anywhere in `order`, keyed by the declaring
    /// entity and attribute name. Later (more specific) entities win.
    fn overrides(&self, order: &[usize]) -> HashMap<(usize, String), Override> {
        let mut overrides: HashMap<(usize, String), Override> = HashMap::new();
        for &i in order {
            let entity = &self.entities[i];
            for redeclaration in &entity.redeclared {
                if let Some(key) = self.declaring(&redeclaration.attribute) {
                    overrides.entry(key).or_default().ty = Some(redeclaration.ty.clone());
                }
            }
            for derived in &entity.derived {
                if let Some(key) = self.declaring(derived) {
                    overrides.entry(key).or_default().derived = true;
                }
            }
        }
        overrides
    }

    /// The entity that declares the explicit attribute `reference` names:
    /// the named supertype itself or its nearest ancestor declaring it.
    fn declaring(&self, reference: &AttributeRef) -> Option<(usize, String)> {
        let order = self.closure(&[&reference.entity])?;
        order
            .iter()
            .rev()
            .copied()
            .find(|&i| {
                self.entities[i]
                    .attributes
                    .iter()
                    .any(|a| a.name == reference.attribute)
            })
            .map(|i| (i, reference.attribute.clone()))
    }

    fn own_slots(
        &self,
        entity: usize,
        overrides: &HashMap<(usize, String), Override>,
    ) -> Vec<Slot> {
        let declaring = &self.entities[entity];
        declaring
            .attributes
            .iter()
            .map(|attribute| {
                let redeclared = overrides.get(&(entity, attribute.name.clone()));
                let ty = redeclared
                    .and_then(|o| o.ty.as_ref())
                    .unwrap_or(&attribute.ty);
                Slot {
                    entity: declaring.name.clone(),
                    name: attribute.name.clone(),
                    optional: attribute.optional,
                    derived: redeclared.is_some_and(|o| o.derived),
                    min_len: self.min_len(ty),
                }
            })
            .collect()
    }
}
