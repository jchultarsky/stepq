//! Prints the surface of every face and the curve of every edge that a
//! shape representation reaches, one JSON object per line, for
//! `tools/verify-read.py surfaces` and `curves` to compare with Open
//! CASCADE.
//!
//! ```console
//! $ cargo run --release --features serde --example geometry -- FILE...
//! ```
//!
//! Faces and edges are found by walking forward from the items of each
//! shape representation, in file order, without entering another
//! representation, a styled item or a mapped item (whose representation is
//! walked on its own). Each
//! face is read with [`Reader::for_item`], in the units of every
//! representation that holds it; each edge with the reader of the first
//! face that reaches it, or of its representation if no face does.
//!
//! Each line holds the file, the face or edge's `rank` (its 1-based
//! position in the file, which is Open CASCADE's entity number), its `#id`,
//! `item` (`face` or `edge`), an edge's `face` (absent for an edge of a
//! wireframe, reached from no face), and either its `surface` or `curve`
//! ([`stepq::geometry::Surface`], [`stepq::geometry::Curve`], in mm and
//! rad) or the `error` that kept it from being read. The format is for
//! that script, not an interface.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use stepq::geometry::{Curve, Reader, Surface};
use stepq::model::Graph;
use stepq::p21::{Exchange, Instance, Param, Record, parse};

#[derive(serde::Serialize)]
struct Line<'a> {
    file: &'a str,
    rank: usize,
    id: u64,
    item: &'static str,
    /// For an edge, the face through which it was reached; `None` for an
    /// edge reached from a representation directly (a wireframe).
    #[serde(skip_serializing_if = "Option::is_none")]
    face: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    surface: Option<Surface>,
    #[serde(skip_serializing_if = "Option::is_none")]
    curve: Option<Curve>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Types whose name ends in REPRESENTATION but which are not
/// representations.
const NOT_REPRESENTATIONS: &[&str] = &[
    "SHAPE_DEFINITION_REPRESENTATION",
    "PROPERTY_DEFINITION_REPRESENTATION",
    "DIMENSIONAL_CHARACTERISTIC_REPRESENTATION",
    "CHARACTERIZED_ITEM_WITHIN_REPRESENTATION",
    "PRESENTED_ITEM_REPRESENTATION",
];

fn names(exchange: &Exchange<'_>, instance: &Instance) -> Vec<String> {
    exchange
        .records(instance)
        .map(|record| String::from_utf8_lossy(record.name()).to_ascii_uppercase())
        .collect()
}

/// The items of a shape representation: a simple instance's second
/// attribute, a complex one's `REPRESENTATION` partial's. `None` if it is not
/// one. Other representations, such as a
/// `constructive_geometry_representation` of supplemental geometry, hold no
/// part of a product's shape, and Open CASCADE does not read them into one.
fn items(exchange: &Exchange<'_>, instance: &Instance) -> Option<Vec<u64>> {
    let names = names(exchange, instance);
    if !names.iter().any(|n| n.ends_with("SHAPE_REPRESENTATION"))
        || names
            .iter()
            .any(|n| n.contains("CONTEXT") || NOT_REPRESENTATIONS.contains(&n.as_str()))
    {
        return None;
    }
    let record: Record<'_> = if instance.is_complex() {
        exchange
            .records(instance)
            .find(|r| r.is("REPRESENTATION"))?
    } else {
        exchange.records(instance).next()?
    };
    Some(
        record
            .param(1)?
            .list()?
            .filter_map(|item| item.reference())
            .collect(),
    )
}

/// Where an edge was reached from.
#[derive(Clone, Copy)]
enum Origin {
    Face(u64),
    Representation(u64),
}

/// The faces and edges `representation`'s items reach, not yet in `seen`,
/// in the order reached.
fn walk(
    exchange: &Exchange<'_>,
    representation: u64,
    items: Vec<u64>,
    seen: &mut HashSet<u64>,
    faces: &mut Vec<u64>,
    edges: &mut Vec<(u64, Origin)>,
) {
    let mut stack: Vec<(u64, Origin)> = items
        .into_iter()
        .rev()
        .map(|item| (item, Origin::Representation(representation)))
        .collect();
    while let Some((id, origin)) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(instance) = exchange.get(id) else {
            continue;
        };
        let names = names(exchange, instance);
        let stops = names.iter().any(|n| {
            n.ends_with("REPRESENTATION")
                || n.ends_with("STYLED_ITEM")
                || n == "MAPPED_ITEM"
                || n.ends_with("CONTEXT")
        });
        if stops {
            continue;
        }
        let first = exchange.records(instance).next();
        let is = |name: &str| !instance.is_complex() && names.first().is_some_and(|n| n == name);
        if is("EDGE_CURVE") {
            edges.push((id, origin));
            continue;
        }
        let (origin, references): (Origin, Vec<u64>) = if is("ADVANCED_FACE") || is("FACE_SURFACE")
        {
            faces.push(id);
            // Only the bounds: the surface is read, not walked.
            let bounds = first
                .and_then(|r| r.param(1))
                .and_then(|p| p.list())
                .map(|list| list.filter_map(|p| p.reference()).collect())
                .unwrap_or_default();
            (Origin::Face(id), bounds)
        } else {
            (origin, exchange.references(instance).collect())
        };
        stack.extend(references.into_iter().rev().map(|id| (id, origin)));
    }
}

/// The `#id` attribute `index` of simple instance `#id`.
fn attribute(exchange: &Exchange<'_>, id: u64, index: usize) -> Option<u64> {
    let instance = exchange.get(id)?;
    if instance.is_complex() {
        return None;
    }
    exchange
        .records(instance)
        .next()?
        .param(index)
        .as_ref()
        .and_then(Param::reference)
}

fn main() -> ExitCode {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: geometry FILE...");
        return ExitCode::from(2);
    }
    let mut out = BufWriter::new(io::stdout().lock());
    let mut status = ExitCode::SUCCESS;
    for file in &files {
        let src = match std::fs::read(file) {
            Ok(src) => src,
            Err(error) => {
                eprintln!("{file}: {error}");
                status = ExitCode::from(2);
                continue;
            }
        };
        let graph = match parse(&src) {
            Ok(exchange) => Graph::build(exchange),
            Err(error) => {
                eprintln!("{file}: {error}");
                status = ExitCode::from(2);
                continue;
            }
        };
        if let Err(error) = print(file, &graph, &mut out) {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    }
    if let Err(error) = out.flush() {
        eprintln!("{error}");
        return ExitCode::from(2);
    }
    status
}

fn print(file: &str, graph: &Graph<'_>, out: &mut impl Write) -> io::Result<()> {
    let exchange = graph.exchange();
    let mut seen = HashSet::new();
    let mut faces = Vec::new();
    let mut edges = Vec::new();
    for instance in exchange.instances() {
        if let Some(items) = items(exchange, instance) {
            walk(
                exchange,
                instance.id,
                items,
                &mut seen,
                &mut faces,
                &mut edges,
            );
        }
    }
    let rank = |id: u64| exchange.position(id).map_or(0, |node| node + 1);
    let mut write = |line: &Line<'_>| -> io::Result<()> {
        serde_json::to_writer(&mut *out, line).map_err(io::Error::from)?;
        writeln!(out)
    };

    faces.sort_by_key(|&id| rank(id));
    let mut readers: HashMap<u64, Result<Reader<'_, '_>, String>> = HashMap::new();
    for &face in &faces {
        let reader = Reader::for_item(graph, face).map_err(|e| e.to_string());
        let read = reader.as_ref().map_err(Clone::clone).and_then(|reader| {
            let surface = attribute(exchange, face, 2)
                .ok_or_else(|| format!("#{face}: no face_geometry reference"))?;
            reader.surface(surface).map_err(|e| e.to_string())
        });
        readers.insert(face, reader);
        let (surface, error) = split(read);
        write(&Line {
            file,
            rank: rank(face),
            id: face,
            item: "face",
            face: None,
            surface,
            curve: None,
            error,
        })?;
    }

    edges.sort_by_key(|&(id, _)| rank(id));
    let mut representations: HashMap<u64, Result<Reader<'_, '_>, String>> = HashMap::new();
    for (edge, origin) in edges {
        let face = match origin {
            Origin::Face(face) => Some(face),
            Origin::Representation(_) => None,
        };
        let reader = match origin {
            Origin::Face(face) => &readers[&face],
            Origin::Representation(representation) => {
                representations.entry(representation).or_insert_with(|| {
                    Reader::for_representation(graph, representation).map_err(|e| e.to_string())
                })
            }
        };
        let read = reader.as_ref().map_err(Clone::clone).and_then(|reader| {
            let curve = attribute(exchange, edge, 3)
                .ok_or_else(|| format!("#{edge}: no edge_geometry reference"))?;
            reader.curve(curve).map_err(|e| e.to_string())
        });
        let (curve, error) = split(read);
        write(&Line {
            file,
            rank: rank(edge),
            id: edge,
            item: "edge",
            face,
            surface: None,
            curve,
            error,
        })?;
    }
    Ok(())
}

fn split<T>(result: Result<T, String>) -> (Option<T>, Option<String>) {
    match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    }
}
