//! Command-line front end for the `stepq` library.

use std::fs;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand, ValueEnum};
use stepq::info::Info;
use stepq::model::Graph;
use stepq::p21::parse;

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
    ///
    /// With --format csv, prints the entity-type histogram only.
    Info {
        /// STEP file to inspect, or `-` for standard input.
        file: PathBuf,
        /// How many entity types to list in the table; 0 lists all.
        #[arg(long, default_value_t = 20)]
        top: usize,
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
        // `stepq info big.stp | head` closes the pipe early; that is not an error.
        Err(err) if is_broken_pipe(&err) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> anyhow::Result<()> {
    match &cli.command {
        Command::Info { file, top } => info(file, cli.format, *top),
        Command::Tree { .. } => not_implemented("tree"),
        Command::Bom { .. } => not_implemented("bom"),
        Command::Split { .. } => not_implemented("split"),
        Command::Lint { .. } => not_implemented("lint"),
    }
}

fn not_implemented(name: &str) -> anyhow::Result<()> {
    bail!("`stepq {name}` is not implemented yet — see ROADMAP.md")
}

fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|io| io.kind() == io::ErrorKind::BrokenPipe)
            || matches!(
                cause.downcast_ref::<stepq::Error>(),
                Some(stepq::Error::Io(io)) if io.kind() == io::ErrorKind::BrokenPipe
            )
    })
}

/// Reads a whole input file, or standard input for `-`.
fn read_input(path: &Path) -> anyhow::Result<Vec<u8>> {
    if path == Path::new("-") {
        let mut src = Vec::new();
        io::stdin()
            .lock()
            .read_to_end(&mut src)
            .context("reading standard input")?;
        Ok(src)
    } else {
        fs::read(path).with_context(|| format!("reading {}", path.display()))
    }
}

fn display_name(path: &Path) -> String {
    if path == Path::new("-") {
        "<stdin>".to_owned()
    } else {
        path.display().to_string()
    }
}

fn info(path: &Path, format: Format, top: usize) -> anyhow::Result<()> {
    let src = read_input(path)?;
    let name = display_name(path);
    let exchange = parse(&src).with_context(|| format!("parsing {name}"))?;
    let info = Info::new(&Graph::build(exchange));

    let mut out = BufWriter::new(io::stdout().lock());
    match format {
        Format::Table => write_info_table(&mut out, &name, &info, top)?,
        Format::Json => {
            #[derive(serde::Serialize)]
            struct Document<'a> {
                file: &'a str,
                #[serde(flatten)]
                info: &'a Info,
            }
            serde_json::to_writer_pretty(
                &mut out,
                &Document {
                    file: &name,
                    info: &info,
                },
            )?;
            writeln!(out)?;
        }
        Format::Csv => {
            writeln!(out, "entity_type,count")?;
            for entity in &info.entity_types {
                writeln!(out, "{},{}", entity.name, entity.count)?;
            }
        }
    }
    out.flush()?;
    Ok(())
}

fn write_info_table(out: &mut impl Write, name: &str, info: &Info, top: usize) -> io::Result<()> {
    let header = &info.header;
    let row = |out: &mut dyn Write, label: &str, value: &str| {
        if value.is_empty() {
            Ok(())
        } else {
            writeln!(out, "{label:<20}  {value}")
        }
    };
    let optional = |value: &Option<String>| value.clone().unwrap_or_default();

    row(
        out,
        "File",
        &format!("{name} ({} bytes)", grouped(info.bytes)),
    )?;
    row(out, "Schema", &header.schemas.join(", "))?;
    row(out, "Name", &optional(&header.name))?;
    row(out, "Time stamp", &optional(&header.time_stamp))?;
    row(out, "Author", &header.author.join(", "))?;
    row(out, "Organization", &header.organization.join(", "))?;
    row(
        out,
        "Originating system",
        &optional(&header.originating_system),
    )?;
    row(out, "Preprocessor", &optional(&header.preprocessor_version))?;
    row(out, "Authorization", &optional(&header.authorization))?;
    row(out, "Description", &header.description.join("; "))?;
    row(
        out,
        "Implementation level",
        &optional(&header.implementation_level),
    )?;

    let units: Vec<String> = [
        ("length", &info.units.length),
        ("plane angle", &info.units.plane_angle),
        ("solid angle", &info.units.solid_angle),
    ]
    .into_iter()
    .filter(|(_, names)| !names.is_empty())
    .map(|(kind, names)| format!("{kind} {}", names.join(", ")))
    .collect();
    row(out, "Units", &units.join("; "))?;

    let sections = if info.sections == 1 {
        "section"
    } else {
        "sections"
    };
    row(
        out,
        "Instances",
        &format!(
            "{} ({} complex) in {} data {sections}",
            grouped(info.instances),
            grouped(info.complex_instances),
            info.sections
        ),
    )?;
    let structure = if info.assembly_usages == 0 {
        "no assembly structure".to_owned()
    } else {
        format!("{} assembly usages", grouped(info.assembly_usages))
    };
    row(
        out,
        "Products",
        &format!("{} ({structure})", grouped(info.products)),
    )?;
    if info.unresolved_references > 0 {
        row(
            out,
            "Warning",
            &format!(
                "{} references to undefined instances",
                grouped(info.unresolved_references)
            ),
        )?;
    }

    writeln!(out)?;
    writeln!(out, "Entity types ({})", info.entity_types.len())?;
    let shown = if top == 0 {
        info.entity_types.len()
    } else {
        top.min(info.entity_types.len())
    };
    for entity in &info.entity_types[..shown] {
        writeln!(out, "{:>10}  {}", grouped(entity.count), entity.name)?;
    }
    let hidden = info.entity_types.len() - shown;
    if hidden > 0 {
        writeln!(out, "{:>10}  … {hidden} more (--top 0 lists all)", "")?;
    }
    Ok(())
}

/// `1234567` as `1,234,567`.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}
