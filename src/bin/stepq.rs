//! Command-line front end for the `stepq` library.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand, ValueEnum};
use stepq::info::Info;
use stepq::model::{Definition, Graph, Placement, ProductStructure, Usage};
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
    ///
    /// Components used several times by the same assembly are shown once with
    /// a count. With --format json, prints every definition and usage; with
    /// --format csv, one row per usage.
    Tree {
        /// STEP file to inspect, or `-` for standard input.
        file: PathBuf,
        /// List every usage separately, with the entities that place it.
        #[arg(long)]
        usages: bool,
    },
    /// Emit a bill of materials: each component under each top-level
    /// assembly, with its total quantity.
    Bom {
        /// STEP file to inspect, or `-` for standard input.
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
        Command::Tree { file, usages } => tree(file, cli.format, *usages),
        Command::Bom { file } => bom(file, cli.format),
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

/// Reads and parses `path` into its product structure.
fn with_structure<T>(
    path: &Path,
    run: impl FnOnce(&ProductStructure) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let src = read_input(path)?;
    let name = display_name(path);
    let exchange = parse(&src).with_context(|| format!("parsing {name}"))?;
    let structure = ProductStructure::new(&Graph::build(exchange));
    run(&structure)
}

fn tree(path: &Path, format: Format, usages: bool) -> anyhow::Result<()> {
    with_structure(path, |structure| {
        let mut out = BufWriter::new(io::stdout().lock());
        match format {
            Format::Table => write_tree(&mut out, structure, usages)?,
            Format::Json => {
                serde_json::to_writer_pretty(&mut out, structure)?;
                writeln!(out)?;
            }
            Format::Csv => write_usages_csv(&mut out, structure)?,
        }
        out.flush()?;
        Ok(())
    })
}

fn write_tree(out: &mut impl Write, structure: &ProductStructure, usages: bool) -> io::Result<()> {
    let definitions = structure.definitions();
    for (i, &root) in structure.roots().iter().enumerate() {
        if i > 0 {
            writeln!(out)?;
        }
        writeln!(out, "{}", definition_label(&definitions[root]))?;
        let mut path = vec![root];
        write_children(out, structure, root, "", usages, &mut path)?;
    }
    if structure.roots().is_empty() && !definitions.is_empty() {
        writeln!(
            out,
            "(no top-level definition: the assembly structure is cyclic)"
        )?;
    }

    writeln!(out)?;
    writeln!(
        out,
        "{} product definitions, {} assembly usages, {} top-level",
        grouped(definitions.len()),
        grouped(structure.usages().len()),
        grouped(structure.roots().len()),
    )?;
    let reversed = structure
        .usages()
        .iter()
        .filter(|u| u.placement.as_ref().and_then(Placement::reversed) == Some(true))
        .count();
    if reversed > 0 {
        writeln!(
            out,
            "note: {reversed} placements name the assembly's shape as rep_1; \
             the assembly usage, not the shape relationship, decides which side is the parent"
        )?;
    }
    Ok(())
}

/// Deeper trees are cut off in the table, so crafted input cannot exhaust
/// the stack.
const MAX_TREE_DEPTH: usize = 256;

fn write_children(
    out: &mut impl Write,
    structure: &ProductStructure,
    definition: usize,
    prefix: &str,
    usages: bool,
    path: &mut Vec<usize>,
) -> io::Result<()> {
    let rows: Vec<(usize, Vec<&Usage>)> = if usages {
        structure
            .children(definition)
            .map(|usage| (usage.child, vec![usage]))
            .collect()
    } else {
        structure.components(definition)
    };
    for (i, (child, group)) in rows.iter().enumerate() {
        let (branch, indent) = if i + 1 == rows.len() {
            ("└── ", "    ")
        } else {
            ("├── ", "│   ")
        };
        let mut line = format!(
            "{prefix}{branch}{}",
            definition_label(&structure.definitions()[*child])
        );
        if usages {
            line.push_str(&usage_detail(group[0]));
        } else {
            let quantity: f64 = group.iter().map(|u| u.quantity.unwrap_or(1.0)).sum();
            if (quantity - 1.0).abs() > f64::EPSILON {
                let _ = write!(line, "  ×{quantity}");
            }
        }
        if path.contains(child) {
            writeln!(out, "{line}  (cycle)")?;
            continue;
        }
        if path.len() >= MAX_TREE_DEPTH {
            writeln!(out, "{line}  (too deep; not expanded)")?;
            continue;
        }
        writeln!(out, "{line}")?;
        path.push(*child);
        write_children(
            out,
            structure,
            *child,
            &format!("{prefix}{indent}"),
            usages,
            path,
        )?;
        path.pop();
    }
    Ok(())
}

/// The product name, with its id when that differs.
fn definition_label(definition: &Definition) -> String {
    let product = definition.product.as_ref();
    let id = product.and_then(|p| p.id.as_deref());
    let name = product
        .and_then(|p| p.name.as_deref())
        .or(id)
        .or(definition.id.as_deref());
    match (name, id) {
        (Some(name), Some(id)) if name != id => format!("{name} [{id}]"),
        (Some(name), _) => name.to_owned(),
        (None, _) => format!("#{}", definition.instance),
    }
}

fn usage_detail(usage: &Usage) -> String {
    let mut detail = format!("  #{}", usage.instance);
    // Writing to a String cannot fail.
    if let Some(name) = &usage.name {
        let _ = write!(detail, " '{name}'");
    }
    if let Some(designator) = &usage.reference_designator {
        let _ = write!(detail, " {designator}");
    }
    if let Some(quantity) = usage.quantity {
        let _ = write!(detail, " ×{quantity}");
    }
    match &usage.placement {
        Some(Placement::ShapeRelationship {
            context_dependent_shape_representation,
            relationship,
            transformation,
            reversed,
            ..
        }) => {
            let _ = write!(
                detail,
                "  placed by #{context_dependent_shape_representation} → #{relationship}"
            );
            if let Some(transformation) = transformation {
                let _ = write!(detail, " (#{transformation})");
            }
            if *reversed == Some(true) {
                detail.push_str(" [rep_1/rep_2 reversed]");
            }
        }
        Some(Placement::MappedItem {
            mapped_item,
            representation_map,
            target,
            ..
        }) => {
            let _ = write!(
                detail,
                "  placed by mapped item #{mapped_item} (map #{representation_map}"
            );
            if let Some(target) = target {
                let _ = write!(detail, ", target #{target}");
            }
            detail.push(')');
        }
        Some(_) => detail.push_str("  (placed)"),
        None => detail.push_str("  (no placement)"),
    }
    detail
}

/// One row per usage. `placement` is the representation relationship for a
/// shape-relationship placement and the `MAPPED_ITEM` for a mapped item;
/// `transformation` is the relationship's transformation or the mapped
/// item's target placement.
fn write_usages_csv(out: &mut impl Write, structure: &ProductStructure) -> io::Result<()> {
    writeln!(
        out,
        "usage,usage_id,parent,parent_product,child,child_product,quantity,reference_designator,placement_kind,placement,transformation,reversed"
    )?;
    let id = |id: Option<u64>| id.map_or_else(String::new, |id| id.to_string());
    let definitions = structure.definitions();
    for usage in structure.usages() {
        let parent = &definitions[usage.parent];
        let child = &definitions[usage.child];
        let (kind, placement, transformation, reversed) = match &usage.placement {
            Some(Placement::ShapeRelationship {
                relationship,
                transformation,
                reversed,
                ..
            }) => (
                "shape_relationship",
                relationship.to_string(),
                id(*transformation),
                reversed.map_or_else(String::new, |r| r.to_string()),
            ),
            Some(Placement::MappedItem {
                mapped_item,
                target,
                ..
            }) => (
                "mapped_item",
                mapped_item.to_string(),
                id(*target),
                String::new(),
            ),
            Some(_) | None => ("", String::new(), String::new(), String::new()),
        };
        writeln!(
            out,
            "{},{},{},{},{},{},{},{},{kind},{placement},{transformation},{reversed}",
            usage.instance,
            csv_field(usage.id.as_deref().unwrap_or_default()),
            parent.instance,
            csv_field(&definition_label(parent)),
            child.instance,
            csv_field(&definition_label(child)),
            usage.quantity.unwrap_or(1.0),
            csv_field(usage.reference_designator.as_deref().unwrap_or_default()),
        )?;
    }
    Ok(())
}

fn bom(path: &Path, format: Format) -> anyhow::Result<()> {
    with_structure(path, |structure| {
        let definitions = structure.definitions();
        let mut out = BufWriter::new(io::stdout().lock());
        match format {
            Format::Table => {
                for (i, &root) in structure.roots().iter().enumerate() {
                    if i > 0 {
                        writeln!(out)?;
                    }
                    writeln!(out, "{}", definition_label(&definitions[root]))?;
                    let lines = structure.bill_of_materials(root);
                    if lines.is_empty() {
                        writeln!(out, "  (no components)")?;
                        continue;
                    }
                    writeln!(out, "{:>10}  {:<8}  PRODUCT", "QUANTITY", "TYPE")?;
                    for line in lines {
                        writeln!(
                            out,
                            "{:>10}  {:<8}  {}",
                            line.quantity,
                            kind(line.is_assembly),
                            definition_label(&definitions[line.definition])
                        )?;
                    }
                }
            }
            Format::Json => {
                #[derive(serde::Serialize)]
                struct Line<'a> {
                    quantity: f64,
                    kind: &'static str,
                    definition: &'a Definition,
                }
                #[derive(serde::Serialize)]
                struct Bom<'a> {
                    root: &'a Definition,
                    lines: Vec<Line<'a>>,
                }
                let boms: Vec<Bom<'_>> = structure
                    .roots()
                    .iter()
                    .map(|&root| Bom {
                        root: &definitions[root],
                        lines: structure
                            .bill_of_materials(root)
                            .into_iter()
                            .map(|line| Line {
                                quantity: line.quantity,
                                kind: kind(line.is_assembly),
                                definition: &definitions[line.definition],
                            })
                            .collect(),
                    })
                    .collect();
                serde_json::to_writer_pretty(&mut out, &boms)?;
                writeln!(out)?;
            }
            Format::Csv => {
                writeln!(out, "root,product_id,product_name,type,quantity")?;
                for &root in structure.roots() {
                    let root_label = definition_label(&definitions[root]);
                    for line in structure.bill_of_materials(root) {
                        let product = definitions[line.definition].product.as_ref();
                        writeln!(
                            out,
                            "{},{},{},{},{}",
                            csv_field(&root_label),
                            csv_field(product.and_then(|p| p.id.as_deref()).unwrap_or_default()),
                            csv_field(product.and_then(|p| p.name.as_deref()).unwrap_or_default()),
                            kind(line.is_assembly),
                            line.quantity
                        )?;
                    }
                }
            }
        }
        out.flush()?;
        Ok(())
    })
}

fn kind(is_assembly: bool) -> &'static str {
    if is_assembly { "assembly" } else { "part" }
}

/// Quotes a CSV field when it contains a comma, quote or line break.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
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
