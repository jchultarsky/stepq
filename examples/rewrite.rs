//! Rewrites a STEP file through stepq's parser and writer.
//!
//! Every instance is copied byte for byte; with `--dense`, instances are
//! renumbered from `#1`. A file with a dangling reference is refused.
//! `tools/verify-rewrite.sh` uses this to show, through Open CASCADE, that
//! writing changes no geometry.
//!
//! ```text
//! cargo run --release --example rewrite -- [--dense] INPUT OUTPUT
//! ```

use std::error::Error;
use std::fs::{self, File};
use std::io::BufWriter;
use std::process::ExitCode;

use stepq::model::Graph;
use stepq::p21::{Numbering, Writer, parse};

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let dense = match args.iter().position(|arg| arg == "--dense") {
        Some(index) => {
            args.remove(index);
            true
        }
        None => false,
    };
    let [input, output] = args.as_slice() else {
        eprintln!("usage: rewrite [--dense] INPUT OUTPUT");
        return ExitCode::from(2);
    };
    match rewrite(input, output, dense) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {input}: {error}");
            ExitCode::FAILURE
        }
    }
}

fn rewrite(input: &str, output: &str, dense: bool) -> Result<(), Box<dyn Error>> {
    let src = fs::read(input)?;
    let graph = Graph::new(parse(&src)?)?;
    let numbering = if dense {
        Numbering::Dense
    } else {
        Numbering::Preserve
    };
    Writer::new(graph.exchange())
        .numbering(numbering)
        .write_all(BufWriter::new(File::create(output)?))?;
    Ok(())
}
