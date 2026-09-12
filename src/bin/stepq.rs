//! Command-line front end for the `stepq` library.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::bail;
use clap::{Parser, Subcommand, ValueEnum};

/// Query, inspect, split and reshape STEP (ISO 10303-21) files.
#[derive(Debug, Parser)]
#[command(name = "stepq", version, about, long_about = None)]
struct Cli {
    /// Output format for commands that produce structured data.
    #[arg(long, global = true, value_enum, default_value_t = Format::Table)]
    format: Format,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    /// Human-readable table.
    Table,
    /// JSON, one document.
    Json,
    /// Comma-separated values.
    Csv,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show header, schema, units and entity statistics.
    Info {
        /// STEP file to inspect.
        file: PathBuf,
    },
    /// Print the assembly hierarchy.
    Tree {
        /// STEP file to inspect.
        file: PathBuf,
    },
    /// Emit a bill of materials.
    Bom {
        /// STEP file to inspect.
        file: PathBuf,
    },
    /// Explode an assembly into one file per sub-assembly and part.
    Split {
        /// STEP file to split.
        file: PathBuf,
        /// Directory to write the output files into.
        #[arg(short, long, default_value = ".")]
        out: PathBuf,
    },
    /// Check a file for structural problems.
    Lint {
        /// STEP file to check.
        file: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> anyhow::Result<()> {
    let name = match cli.command {
        Command::Info { .. } => "info",
        Command::Tree { .. } => "tree",
        Command::Bom { .. } => "bom",
        Command::Split { .. } => "split",
        Command::Lint { .. } => "lint",
    };
    bail!("`stepq {name}` is not implemented yet — see ROADMAP.md")
}
