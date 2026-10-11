//! Prints every occurrence under every root of a file, one JSON object per
//! line, for `tools/verify-read.py occurrences` to compare with Open
//! CASCADE.
//!
//! ```console
//! $ cargo run --release --features serde --example occurrences -- FILE...
//! ```
//!
//! Each line holds the file, the `#id` of the root definition, and for the
//! occurrence: its `path` of usage `#id`s from the root down, each usage's
//! `id`, `name` and `description` (`usages`), the `id` and `name` of the
//! product at each level, the root's first (`products`), the `#id` of its
//! `definition` and of its `representation` (absent if there is none), and
//! either its `matrix` (the 3×4 transform to the root, row by row, in mm)
//! or the `error` ([`stepq::model::PlacementError`], its variant under
//! `kind`) with its `reason` in words. Strings that are blank or unset are
//! `null`, and a usage's absent `description` is blank. The format is for
//! that script, not an interface.

use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use stepq::model::{Graph, PlacementError, Placements, ProductStructure};
use stepq::p21::parse;

#[derive(serde::Serialize)]
struct Line<'a> {
    file: &'a str,
    root: u64,
    path: Vec<u64>,
    usages: Vec<Names<'a>>,
    products: Vec<Names<'a>>,
    definition: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    representation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    matrix: Option<[f64; 12]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a PlacementError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(serde::Serialize)]
struct Names<'a> {
    id: Option<&'a str>,
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
}

fn main() -> ExitCode {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: occurrences FILE...");
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
        let exchange = match parse(&src) {
            Ok(exchange) => exchange,
            Err(error) => {
                eprintln!("{file}: {error}");
                status = ExitCode::from(2);
                continue;
            }
        };
        let graph = Graph::build(exchange);
        let structure = ProductStructure::new(&graph);
        let placements = Placements::new(&graph, &structure);
        let definitions = structure.definitions();
        let usages = structure.usages();
        let product = |definition: usize| {
            let product = definitions[definition].product.as_ref();
            Names {
                id: product.and_then(|p| p.id.as_deref()),
                name: product.and_then(|p| p.name.as_deref()),
                description: None,
            }
        };
        for &root in structure.roots() {
            for occurrence in placements.occurrences(root) {
                let path = occurrence.path.iter().map(|&u| &usages[u]);
                let line = Line {
                    file,
                    root: definitions[root].instance,
                    path: path.clone().map(|u| u.instance).collect(),
                    usages: path
                        .clone()
                        .map(|u| Names {
                            id: u.id.as_deref(),
                            name: u.name.as_deref(),
                            description: u.description.as_deref(),
                        })
                        .collect(),
                    products: std::iter::once(root)
                        .chain(path.map(|u| u.child))
                        .map(product)
                        .collect(),
                    definition: definitions[occurrence.definition].instance,
                    representation: occurrence.representation,
                    matrix: occurrence.transform.as_ref().ok().map(|t| {
                        let (r, t) = (t.rotation, t.translation);
                        [
                            r[0][0], r[0][1], r[0][2], t[0], r[1][0], r[1][1], r[1][2], t[1],
                            r[2][0], r[2][1], r[2][2], t[2],
                        ]
                    }),
                    error: occurrence.transform.as_ref().err(),
                    reason: occurrence.transform.as_ref().err().map(ToString::to_string),
                };
                let written = serde_json::to_writer(&mut out, &line)
                    .map_err(io::Error::from)
                    .and_then(|()| writeln!(out));
                if let Err(error) = written {
                    eprintln!("{error}");
                    return ExitCode::from(2);
                }
            }
        }
    }
    if let Err(error) = out.flush() {
        eprintln!("{error}");
        return ExitCode::from(2);
    }
    status
}
