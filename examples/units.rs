//! Prints the units of every representation context that assigns any, one
//! JSON object per line, for `tools/verify-read.py units` to compare with
//! Open CASCADE.
//!
//! ```console
//! $ cargo run --release --features serde --example units -- FILE...
//! ```
//!
//! Each line holds the file, the context's `rank` (its 1-based position in
//! the file, which is Open CASCADE's entity number), its `#id` and entity
//! types, and either its `units` ([`stepq::units::ContextUnits`], each kind
//! as `{"ok": unit}` or `{"error": reason}`) or the `error` that kept it
//! from being read at all. The format is for that script, not an interface.

use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use stepq::model::Graph;
use stepq::p21::parse;
use stepq::units::{ContextUnits, UnitError};

#[derive(serde::Serialize)]
struct Line<'a> {
    file: &'a str,
    rank: usize,
    context: u64,
    entity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    units: Option<ContextUnits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<UnitError>,
}

fn main() -> ExitCode {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: units FILE...");
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
        let exchange = graph.exchange();
        for node in exchange.instances_of("GLOBAL_UNIT_ASSIGNED_CONTEXT") {
            let instance = graph.instance(node);
            let entity = exchange
                .records(instance)
                .map(|record| String::from_utf8_lossy(record.name()).to_ascii_uppercase())
                .collect::<Vec<_>>()
                .join("+");
            let (units, error) = match ContextUnits::of_context(&graph, instance.id) {
                Ok(units) => (Some(units), None),
                Err(error) => (None, Some(error)),
            };
            let line = Line {
                file,
                rank: node + 1,
                context: instance.id,
                entity,
                units,
                error,
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
    if let Err(error) = out.flush() {
        eprintln!("{error}");
        return ExitCode::from(2);
    }
    status
}
