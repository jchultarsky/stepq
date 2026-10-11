//! Prints the shape of every product definition, one JSON object per line,
//! for `tools/verify-read.py shapes` to compare with Open CASCADE.
//!
//! ```console
//! $ cargo run --release --features serde --example shapes -- FILE...
//! ```
//!
//! Each line holds the file, the definition's `#id`, its product's id and
//! name, the `#id`s of its own solids ([`ProductStructure::shape`]), the
//! entity types of its other items, what was not followed, and its subtree
//! total: its own solids, plus each usage's child's total, so a component
//! used twice counts twice; and its subtree's `tessellated_solid`s, counted
//! the same way, which Open CASCADE reads as solids. A definition more than 1,024 levels deep, or on
//! a cycle, adds nothing to the total. The format is for that script, not an
//! interface.

use std::collections::BTreeMap;
use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use stepq::model::{DefinitionShape, Graph, NotFollowed, ProductStructure};
use stepq::p21::parse;

#[derive(serde::Serialize)]
struct Line<'a> {
    file: &'a str,
    definition: u64,
    product_id: Option<&'a str>,
    product_name: Option<&'a str>,
    solids: Vec<u64>,
    other: BTreeMap<&'a str, usize>,
    not_followed: &'a [NotFollowed],
    total: usize,
    /// The subtree's `tessellated_solid`s, counted as `total` counts solids.
    tessellated: usize,
}

fn main() -> ExitCode {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: shapes FILE...");
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

/// Own solids plus each usage's child's total, memoised; a definition on
/// its own path, or too deep, adds nothing.
fn total(
    structure: &ProductStructure,
    own: &[usize],
    totals: &mut [Option<usize>],
    on_path: &mut Vec<usize>,
    definition: usize,
) -> usize {
    if let Some(total) = totals[definition] {
        return total;
    }
    if on_path.contains(&definition) || on_path.len() > 1024 {
        return 0;
    }
    on_path.push(definition);
    let mut sum = own[definition];
    for usage in structure.children(definition) {
        sum += total(structure, own, totals, on_path, usage.child);
    }
    on_path.pop();
    totals[definition] = Some(sum);
    sum
}

fn print(file: &str, graph: &Graph<'_>, out: &mut impl Write) -> io::Result<()> {
    let structure = ProductStructure::new(graph);
    let shapes: Vec<DefinitionShape> = (0..structure.definitions().len())
        .map(|definition| structure.shape(graph, definition))
        .collect();
    let solids: Vec<usize> = shapes.iter().map(|shape| shape.solids.len()).collect();
    let tessellated: Vec<usize> = shapes
        .iter()
        .map(|shape| {
            let is = |item: &&stepq::model::ShapeItem| item.entity == "TESSELLATED_SOLID";
            shape.other.iter().filter(is).count()
        })
        .collect();
    let mut totals = vec![None; shapes.len()];
    let mut tessellated_totals = vec![None; shapes.len()];
    for (index, definition) in structure.definitions().iter().enumerate() {
        let shape = &shapes[index];
        let mut other = BTreeMap::new();
        for item in &shape.other {
            *other.entry(item.entity.as_str()).or_insert(0) += 1;
        }
        let product = definition.product.as_ref();
        let line = Line {
            file,
            definition: definition.instance,
            product_id: product.and_then(|p| p.id.as_deref()),
            product_name: product.and_then(|p| p.name.as_deref()),
            solids: shape.solids.iter().map(|item| item.item).collect(),
            other,
            not_followed: &shape.not_followed,
            total: total(&structure, &solids, &mut totals, &mut Vec::new(), index),
            tessellated: total(
                &structure,
                &tessellated,
                &mut tessellated_totals,
                &mut Vec::new(),
                index,
            ),
        };
        serde_json::to_writer(&mut *out, &line).map_err(io::Error::from)?;
        writeln!(out)?;
    }
    Ok(())
}
