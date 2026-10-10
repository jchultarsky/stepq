//! Prints the topology of every solid in a file, one JSON object per line,
//! for `tools/verify-read.py topology` to compare with Open CASCADE.
//!
//! ```console
//! $ cargo run --release --features serde --example topology -- FILE...
//! ```
//!
//! Every instance that is, or has a partial that is, a
//! `manifold_solid_brep`, `brep_with_voids`, `faceted_brep`, surface model
//! or `tessellated_solid` is read with [`Solid::read`], in file order (all
//! but the first two are refused as unsupported). Each line holds the
//! file, the solid's `rank` (its 1-based position in the file, which is Open
//! CASCADE's entity number), its `#id` and entity, and either the `error`
//! that kept it from being read or its faces, edges and vertex count:
//! each face's rank and `face_sense`, and each edge's rank, the ranks of
//! its start and end vertices, `same_sense`, whether it is a seam, and its
//! uses as (face rank, `forward`). The format is for that script, not an
//! interface.

use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use stepq::model::Graph;
use stepq::p21::{Exchange, parse};
use stepq::topology::Solid;

/// The solids and models whose topology is asked for.
const SOLIDS: &[&str] = &[
    "MANIFOLD_SOLID_BREP",
    "BREP_WITH_VOIDS",
    "FACETED_BREP",
    "SHELL_BASED_SURFACE_MODEL",
    "FACE_BASED_SURFACE_MODEL",
    "TESSELLATED_SOLID",
];

#[derive(serde::Serialize)]
struct Line<'a> {
    file: &'a str,
    rank: usize,
    id: u64,
    entity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    faces: Vec<FaceLine>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    edges: Vec<EdgeLine>,
    vertices: usize,
}

#[derive(serde::Serialize)]
struct FaceLine {
    rank: usize,
    face_sense: bool,
    bounds: usize,
}

#[derive(serde::Serialize)]
struct EdgeLine {
    rank: usize,
    start: usize,
    end: usize,
    same_sense: bool,
    seam: bool,
    /// (face rank, forward) for each use.
    uses: Vec<(usize, bool)>,
}

fn main() -> ExitCode {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: topology FILE...");
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

fn entity(exchange: &Exchange<'_>, id: u64) -> String {
    exchange.get(id).map_or_else(String::new, |instance| {
        exchange
            .records(instance)
            .map(|record| String::from_utf8_lossy(record.name()).to_ascii_uppercase())
            .collect::<Vec<_>>()
            .join("+")
    })
}

fn print(file: &str, graph: &Graph<'_>, out: &mut impl Write) -> io::Result<()> {
    let exchange = graph.exchange();
    let rank = |id: u64| exchange.position(id).map_or(0, |node| node + 1);
    for instance in exchange.instances() {
        if !exchange
            .records(instance)
            .any(|record| SOLIDS.iter().any(|name| record.is(name)))
        {
            continue;
        }
        let id = instance.id;
        let mut line = Line {
            file,
            rank: rank(id),
            id,
            entity: entity(exchange, id),
            error: None,
            error_kind: None,
            faces: Vec::new(),
            edges: Vec::new(),
            vertices: 0,
        };
        match Solid::read(graph, id) {
            Err(error) => {
                line.error_kind = Some(match error {
                    stepq::topology::TopologyError::Unsupported { .. } => "unsupported",
                    stepq::topology::TopologyError::Malformed { .. } => "malformed",
                    _ => "other",
                });
                line.error = Some(error.to_string());
            }
            Ok(solid) => {
                let vertex = |index: usize| rank(solid.vertices()[index].instance);
                line.faces = (0..solid.faces().len())
                    .map(|face| FaceLine {
                        rank: rank(solid.faces()[face].instance),
                        face_sense: solid.face_sense(face),
                        bounds: solid.faces()[face].bounds.len(),
                    })
                    .collect();
                line.edges = solid
                    .edges()
                    .iter()
                    .enumerate()
                    .map(|(index, edge)| EdgeLine {
                        rank: rank(edge.instance),
                        start: vertex(edge.start),
                        end: vertex(edge.end),
                        same_sense: edge.same_sense,
                        seam: solid.is_seam(index),
                        uses: solid
                            .uses(index)
                            .iter()
                            .map(|used| (rank(solid.faces()[used.face].instance), used.forward))
                            .collect(),
                    })
                    .collect();
                line.vertices = solid.vertices().len();
            }
        }
        serde_json::to_writer(&mut *out, &line).map_err(io::Error::from)?;
        writeln!(out)?;
    }
    Ok(())
}
