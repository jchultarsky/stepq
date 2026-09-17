//! Resolving external references: the inverse of `stepq split --master`.
//!
//! This is what `stepq assemble` writes. A master file keeps, of each
//! component, a stub — product, formation, definition, shape definition and
//! a shape without geometry — and a CAx-IF external reference
//! (`applied_document_reference` to a `document_file`) naming the file that
//! holds the component. Assembling replaces every stub with the component
//! file's own instances, renamed after the master's, points every reference
//! to a stub at the component's counterpart, and drops the reference
//! entities. Component files that are masters themselves are resolved the
//! same way, and a file several masters refer to — a part used in two
//! sub-assemblies — is merged once. Nothing is evaluated: instances are
//! copied as written.

use std::collections::{HashMap, HashSet};
use std::io;

use crate::error::{Error, Result};
use crate::model::{Graph, ProductStructure};
use crate::p21::{Exchange, Numbering, Replacements, TokenKind, Writer, parse};
use crate::props::text;

/// The result of [`assemble`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Assembled {
    /// The self-contained file.
    pub text: Vec<u8>,
    /// Every file merged in, once each, as the references name them, in
    /// the order read.
    pub files: Vec<String>,
}

/// Reads a referenced file, given its name as the reference writes it.
pub type Load<'a> = dyn FnMut(&str) -> io::Result<Vec<u8>> + 'a;

/// Assembles the master file in `src`, reading each file it refers to with
/// `load`. A file without external references comes back unchanged.
///
/// # Errors
///
/// Returns [`Error::ExternalReference`] if a file cannot be loaded, holds no
/// product definition, or files refer to each other in a cycle; parse and
/// graph errors of any file; and [`Error::UnresolvedReference`] if a stub is
/// referred to in a way this cannot redirect.
pub fn assemble(src: &[u8], load: &mut Load<'_>) -> Result<Assembled> {
    let graph = Graph::new(parse(src)?)?;
    let exchange = graph.exchange();
    if links(&graph).is_empty() {
        return Ok(Assembled {
            text: src.to_vec(),
            files: Vec::new(),
        });
    }
    let mut merger = Merger {
        load,
        next: max_id(exchange),
        merged: HashMap::new(),
        active: Vec::new(),
        out: Vec::new(),
        files: Vec::new(),
    };
    let (kept, replacements) = merger.resolve(&graph)?;
    let mut out = Vec::new();
    Writer::new(exchange)
        .replacements(&replacements)
        .append(&merger.out)
        .write_selection(kept, &mut out)?;
    Ok(Assembled {
        text: out,
        files: merger.files,
    })
}

/// The instances, already renamed, that stand in for a stub.
#[derive(Debug, Clone, Copy)]
struct Target {
    definition: u64,
    product: Option<u64>,
    formation: Option<u64>,
    shape: Option<u64>,
    shape_link: Option<u64>,
    representation: Option<u64>,
}

/// A product definition of a merged file, by its product's id.
struct Candidate {
    product: Option<String>,
    root: bool,
    target: Target,
}

struct Merger<'l, 'a> {
    load: &'l mut Load<'a>,
    /// The next free offset: every name written so far is at most this.
    next: u64,
    merged: HashMap<String, Vec<Candidate>>,
    /// The files being merged, to catch cycles.
    active: Vec<String>,
    /// The merged files' instances.
    out: Vec<u8>,
    files: Vec<String>,
}

impl Merger<'_, '_> {
    /// Merges every file `graph` refers to and returns the nodes of `graph`
    /// to keep, with its references to stubs redirected.
    fn resolve(&mut self, graph: &Graph<'_>) -> Result<(Vec<usize>, Replacements)> {
        let exchange = graph.exchange();
        let structure = ProductStructure::new(graph);
        let mut dropped: HashSet<usize> = HashSet::new();
        let mut rename: HashMap<u64, u64> = HashMap::new();
        for link in links(graph) {
            dropped.extend(link.chain.iter().copied());
            let stub = structure
                .definitions()
                .iter()
                .find(|definition| definition.instance == link.stub)
                .ok_or_else(|| Error::ExternalReference {
                    file: link.file.clone(),
                    message: format!("#{} is not a product definition", link.stub),
                })?;
            let wanted = stub.product.as_ref().and_then(|p| p.id.as_deref());
            let target = self.merge(&link.file, wanted)?;

            let mut pair = |stub: Option<u64>, component: Option<u64>| {
                if let (Some(stub), Some(component)) = (stub, component) {
                    rename.insert(stub, component);
                    if let Some(node) = graph.node(stub) {
                        dropped.insert(node);
                    }
                }
            };
            pair(Some(stub.instance), Some(target.definition));
            pair(stub.product.as_ref().map(|p| p.instance), target.product);
            pair(formation(exchange, stub.instance), target.formation);
            for (shape, links) in shape_definitions(graph, stub.instance) {
                pair(Some(shape), target.shape);
                for sdr in links {
                    pair(Some(sdr), target.shape_link);
                }
            }
            for &rep in &stub.shape_representations {
                pair(Some(rep), target.representation);
            }
        }

        let mut replacements = Replacements::new();
        let kept: Vec<usize> = (0..graph.len())
            .filter(|node| !dropped.contains(node))
            .collect();
        for &node in &kept {
            for token in exchange.instance_tokens(graph.instance(node)) {
                if let TokenKind::InstanceName(id) = token.kind {
                    if let Some(new) = rename.get(&id) {
                        replacements.replace(token.span, format!("#{new}"));
                    }
                }
            }
        }
        Ok((kept, replacements))
    }

    /// Merges `file` unless it already is, and returns the renamed product
    /// definition whose product id is `wanted`, or else its first root.
    fn merge(&mut self, file: &str, wanted: Option<&str>) -> Result<Target> {
        let failed = |message: String| Error::ExternalReference {
            file: file.to_owned(),
            message,
        };
        if !self.merged.contains_key(file) {
            if self.active.iter().any(|active| active == file) {
                return Err(failed("files refer to each other in a cycle".to_owned()));
            }
            let bytes = (self.load)(file).map_err(|error| failed(error.to_string()))?;
            let graph = Graph::new(parse(&bytes)?)?;
            let exchange = graph.exchange();
            let offset = self.next;
            self.next += max_id(exchange);
            self.files.push(file.to_owned());
            self.active.push(file.to_owned());
            let (kept, replacements) = self.resolve(&graph)?;
            self.active.pop();
            Writer::new(exchange)
                .numbering(Numbering::Offset(offset))
                .replacements(&replacements)
                .write_instances(kept, &mut self.out)?;
            self.merged
                .insert(file.to_owned(), candidates(&graph, offset));
        }
        let candidates = &self.merged[file];
        candidates
            .iter()
            .find(|candidate| candidate.product.as_deref() == wanted)
            .or_else(|| candidates.iter().find(|candidate| candidate.root))
            .map(|candidate| candidate.target)
            .ok_or_else(|| failed("holds no product definition".to_owned()))
    }
}

/// Every product definition of a file about to be merged at `offset`.
fn candidates(graph: &Graph<'_>, offset: u64) -> Vec<Candidate> {
    let exchange = graph.exchange();
    let structure = ProductStructure::new(graph);
    let renamed = |id: Option<u64>| id.map(|id| id + offset);
    structure
        .definitions()
        .iter()
        .enumerate()
        .map(|(index, definition)| {
            let shapes = shape_definitions(graph, definition.instance);
            let first = shapes.first();
            Candidate {
                product: definition
                    .product
                    .as_ref()
                    .and_then(|p| p.id.as_deref())
                    .map(str::to_owned),
                root: structure.roots().contains(&index),
                target: Target {
                    definition: definition.instance + offset,
                    product: renamed(definition.product.as_ref().map(|p| p.instance)),
                    formation: renamed(formation(exchange, definition.instance)),
                    shape: renamed(first.map(|(shape, _)| *shape)),
                    shape_link: renamed(first.and_then(|(_, links)| links.first().copied())),
                    representation: renamed(definition.shape_representations.first().copied()),
                },
            }
        })
        .collect()
}

/// One external reference: the file, the stub definition it stands for,
/// and the nodes of the reference entities.
struct Link {
    file: String,
    stub: u64,
    chain: Vec<usize>,
}

/// Every `applied_document_reference` to a `document_file`, per item.
fn links(graph: &Graph<'_>) -> Vec<Link> {
    let exchange = graph.exchange();
    let first = |node: usize| exchange.records(graph.instance(node)).next();
    // A node referred to by nothing but `owner` belongs to the reference.
    let only_from = |node: usize, owner: usize| graph.referenced_by(node) == [owner];
    let mut links = Vec::new();
    for node in 0..graph.len() {
        let Some(reference) = first(node).filter(|r| r.is("APPLIED_DOCUMENT_REFERENCE")) else {
            continue;
        };
        if graph.instance(node).is_complex() {
            continue;
        }
        let Some(document) = reference
            .param(0)
            .and_then(|p| p.reference())
            .and_then(|id| graph.node(id))
        else {
            continue;
        };
        let Some(file_record) = exchange
            .records(graph.instance(document))
            .find(|r| r.is("DOCUMENT_FILE"))
        else {
            continue;
        };
        let mut file = text(file_record.param(0)).unwrap_or_default();
        let mut chain = vec![node, document];
        chain.extend(
            graph
                .references(document)
                .iter()
                .copied()
                .filter(|&kind| only_from(kind, document)),
        );
        for &user in graph.referenced_by(document) {
            let Some(record) = first(user) else {
                continue;
            };
            if record.is("APPLIED_EXTERNAL_IDENTIFICATION_ASSIGNMENT") {
                if let Some(id) =
                    text(record.param(0)).filter(|id| !id.is_empty() && !id.starts_with('#'))
                {
                    file = id;
                }
                chain.push(user);
                chain.extend(
                    graph
                        .references(user)
                        .iter()
                        .copied()
                        .filter(|&part| part != document && only_from(part, user)),
                );
            } else if record.is("DOCUMENT_REPRESENTATION_TYPE")
                || (record.is("PROPERTY_DEFINITION")
                    && text(record.param(0)).as_deref() == Some("external definition"))
            {
                chain.push(user);
                chain.extend(graph.referenced_by(user).iter().copied().filter(|&link| {
                    first(link).is_some_and(|r| r.is("PROPERTY_DEFINITION_REPRESENTATION"))
                }));
            }
        }
        for &user in graph.referenced_by(node) {
            if first(user).is_some_and(|r| r.is("ROLE_ASSOCIATION")) {
                chain.push(user);
                chain.extend(
                    graph
                        .references(user)
                        .iter()
                        .copied()
                        .filter(|&role| role != node && only_from(role, user)),
                );
            }
        }
        let items = reference
            .param(2)
            .and_then(|p| p.list())
            .into_iter()
            .flatten()
            .filter_map(|p| p.reference());
        for stub in items {
            links.push(Link {
                file: file.clone(),
                stub,
                chain: chain.clone(),
            });
        }
    }
    links
}

fn max_id(exchange: &Exchange<'_>) -> u64 {
    exchange.instances().iter().map(|i| i.id).max().unwrap_or(0)
}

/// The formation a product definition names, attribute 2.
fn formation(exchange: &Exchange<'_>, definition: u64) -> Option<u64> {
    let record = exchange.records(exchange.get(definition)?).next()?;
    record.param(2)?.reference()
}

/// Each `product_definition_shape` of `definition`, with the
/// `shape_definition_representation`s of it.
fn shape_definitions(graph: &Graph<'_>, definition: u64) -> Vec<(u64, Vec<u64>)> {
    let exchange = graph.exchange();
    let points_at = |node: usize, entity: &str, index: usize, target: u64| {
        exchange
            .records(graph.instance(node))
            .next()
            .is_some_and(|record| {
                record.is(entity) && record.param(index).and_then(|p| p.reference()) == Some(target)
            })
    };
    let Some(node) = graph.node(definition) else {
        return Vec::new();
    };
    graph
        .referenced_by(node)
        .iter()
        .copied()
        .filter(|&shape| points_at(shape, "PRODUCT_DEFINITION_SHAPE", 2, definition))
        .map(|shape| {
            let id = graph.instance(shape).id;
            let links = graph
                .referenced_by(shape)
                .iter()
                .copied()
                .filter(|&sdr| points_at(sdr, "SHAPE_DEFINITION_REPRESENTATION", 0, id))
                .map(|sdr| graph.instance(sdr).id)
                .collect();
            (id, links)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A master written by `split --master`: assembly A with a stub of part
    /// P (#20–#25) and its external reference (#52–#62).
    const MASTER: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('AUTOMOTIVE_DESIGN'));ENDSEC;DATA;
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
#25=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#34),#5);
#34=AXIS2_PLACEMENT_3D('',#35,$,$);
#35=CARTESIAN_POINT('',(0.,0.,0.));
#40=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u1','P1','',#10,#20,$);
#41=PRODUCT_DEFINITION_SHAPE('','',#40);
#42=(REPRESENTATION_RELATIONSHIP('','',#25,#15)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#43)SHAPE_REPRESENTATION_RELATIONSHIP());
#43=ITEM_DEFINED_TRANSFORMATION('','',#34,#34);
#44=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#42,#41);
#52=DOCUMENT_TYPE('geometry');
#53=DOCUMENT_FILE('P.stp','',$,#52,'',$);
#54=DOCUMENT_REPRESENTATION_TYPE('digital',#53);
#55=IDENTIFICATION_ROLE('external document id and location',$);
#56=EXTERNAL_SOURCE(IDENTIFIER(''));
#57=APPLIED_EXTERNAL_IDENTIFICATION_ASSIGNMENT('P.stp',#55,#56,(#53));
#58=APPLIED_DOCUMENT_REFERENCE(#53,'',(#20));
#59=OBJECT_ROLE('mandatory',$);
#60=ROLE_ASSOCIATION(#59,#58);
#61=PROPERTY_DEFINITION('external definition',$,#53);
#62=PROPERTY_DEFINITION_REPRESENTATION(#61,#25);
ENDSEC;END-ISO-10303-21;";

    const PART: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('AUTOMOTIVE_DESIGN'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('design');
#2=APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2001,#1);
#3=PRODUCT_CONTEXT('',#1,'mechanical');
#4=PRODUCT_DEFINITION_CONTEXT('part definition',#1,'design');
#5=GEOMETRIC_REPRESENTATION_CONTEXT(3);
#6=PRODUCT('P','plate','',(#3));
#7=PRODUCT_DEFINITION_FORMATION('1','',#6);
#8=PRODUCT_DEFINITION('p','',#7,#4);
#9=PRODUCT_DEFINITION_SHAPE('','',#8);
#10=SHAPE_DEFINITION_REPRESENTATION(#9,#11);
#11=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#12,#14),#5);
#12=AXIS2_PLACEMENT_3D('',#13,$,$);
#13=CARTESIAN_POINT('',(0.,0.,0.));
#14=MANIFOLD_SOLID_BREP('plate',#15);
#15=CLOSED_SHELL('',());
ENDSEC;END-ISO-10303-21;";

    fn loader(name: &str) -> io::Result<Vec<u8>> {
        match name {
            "P.stp" => Ok(PART.as_bytes().to_vec()),
            _ => Err(io::Error::new(io::ErrorKind::NotFound, "no such file")),
        }
    }

    #[test]
    fn stubs_are_replaced_by_the_component_file() {
        let assembled = assemble(MASTER.as_bytes(), &mut loader).unwrap();
        assert_eq!(assembled.files, ["P.stp"]);
        let text = String::from_utf8(assembled.text).unwrap();
        // The part is renamed after the master's highest name, #62.
        for expected in [
            "#40=NEXT_ASSEMBLY_USAGE_OCCURRENCE('u1','P1','',#10,#70,$);",
            "#42=(REPRESENTATION_RELATIONSHIP('','',#73,#15)",
            "#68=PRODUCT('P','plate','',(#65));",
            "#70=PRODUCT_DEFINITION('p','',#69,#66);",
            "#76=MANIFOLD_SOLID_BREP('plate',#77);",
        ] {
            assert!(text.contains(expected), "missing {expected}\n{text}");
        }
        for gone in [
            "#20=",
            "#21=",
            "#22=",
            "#23=",
            "#24=",
            "#25=",
            "DOCUMENT_FILE",
            "DOCUMENT_TYPE",
            "APPLIED_DOCUMENT_REFERENCE",
            "ROLE_ASSOCIATION",
            "OBJECT_ROLE",
            "IDENTIFICATION_ROLE",
            "EXTERNAL_SOURCE",
            "external definition",
        ] {
            assert!(!text.contains(gone), "{gone} is still there\n{text}");
        }

        let graph = Graph::new(parse(text.as_bytes()).unwrap()).unwrap();
        let structure = ProductStructure::new(&graph);
        assert_eq!(structure.usages().len(), 1);
        let child = &structure.definitions()[structure.usages()[0].child];
        assert_eq!(child.product.as_ref().unwrap().id.as_deref(), Some("P"));
        assert_eq!(child.shape_representations, [73]);
    }

    #[test]
    fn a_file_without_references_comes_back_unchanged() {
        let assembled = assemble(PART.as_bytes(), &mut loader).unwrap();
        assert_eq!(assembled.text, PART.as_bytes());
        assert!(assembled.files.is_empty());
    }

    #[test]
    fn missing_files_and_cycles_are_errors() {
        let mut missing = |_: &str| Err(io::Error::new(io::ErrorKind::NotFound, "no such file"));
        let error = assemble(MASTER.as_bytes(), &mut missing).unwrap_err();
        assert_eq!(
            error.to_string(),
            "external reference to P.stp: no such file"
        );

        let mut itself = |_: &str| Ok(MASTER.as_bytes().to_vec());
        let error = assemble(MASTER.as_bytes(), &mut itself).unwrap_err();
        assert_eq!(
            error.to_string(),
            "external reference to P.stp: files refer to each other in a cycle"
        );
    }
}
