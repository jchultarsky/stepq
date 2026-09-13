//! Parses arbitrary bytes and exercises everything built on the result.
//!
//! Invariants: no panic in parsing, graph building, the query views or
//! `Info`; and for a file without dangling references, the writer's output
//! in both numbering modes parses back with the same number of instances.

#![no_main]

use libfuzzer_sys::fuzz_target;
use stepq::info::Info;
use stepq::model::Graph;
use stepq::p21::{Numbering, Params, Writer, parse};

fuzz_target!(|data: &[u8]| {
    let Ok(exchange) = parse(data) else {
        return;
    };
    let count = exchange.instances().len();
    let graph = Graph::build(exchange);
    let _ = Info::new(&graph);

    let exchange = graph.exchange();
    for record in exchange.header() {
        walk(record.params());
    }
    for instance in exchange.instances() {
        let _ = exchange.references(instance).count();
        for record in exchange.records(instance) {
            walk(record.params());
        }
    }
    for node in 0..graph.len() {
        let _ = (graph.references(node), graph.referenced_by(node));
    }

    if !graph.unresolved().is_empty() {
        return;
    }
    for numbering in [Numbering::Preserve, Numbering::Dense] {
        let mut out = Vec::new();
        Writer::new(exchange)
            .numbering(numbering)
            .write_all(&mut out)
            .expect("writing a file without dangling references succeeds");
        let reparsed = parse(&out).expect("writer output parses");
        assert_eq!(reparsed.instances().len(), count, "{numbering:?}");
    }
});

/// Visits every parameter, recursing into lists and typed values. Depth
/// is bounded by the parser's nesting limit.
fn walk(params: Params<'_>) {
    for param in params {
        if let Some(items) = param.list() {
            walk(items);
        }
        if let Some(record) = param.typed() {
            walk(record.params());
        }
        if let Some(literal) = param.literal() {
            let _ = (
                literal.decode(),
                literal.to_i64(),
                literal.to_f64(),
                literal.enumeration(),
            );
        }
    }
}
