//! Command-line front end for the `stepq` library.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand, ValueEnum};
use stepq::info::Info;
use stepq::model::{
    BomNode, BomTreeOptions, Definition, Graph, Placement, ProductStructure, Usage,
};
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
    /// Indented tree: the multi-level view of commands that have one
    /// (`bom`). Other commands print their table.
    Tree,
}

/// Characters used to draw a tree.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum Charset {
    /// UTF-8 box-drawing characters when the locale is UTF-8, ASCII otherwise.
    Auto,
    /// UTF-8 box-drawing characters.
    Utf8,
    /// ASCII only.
    Ascii,
}

/// How each line of a tree starts.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum Prefix {
    /// Tree lines showing the hierarchy.
    Indent,
    /// The line's level number, 0 for the top assembly.
    Depth,
    /// Nothing.
    None,
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
    /// Emit the bill of materials of each top-level assembly.
    ///
    /// By default a multi-level (indented) BOM: every component under its
    /// assembly with its quantity per assembly and, where different, its
    /// total quantity. As a table or tree it is drawn as a tree, with
    /// repeated sub-assemblies expanded once and marked (*); as CSV, one row
    /// per line with its level and item number; as JSON, nested. With
    /// --flat, one line per distinct component with its total quantity.
    Bom {
        /// STEP file to inspect, or `-` for standard input.
        file: PathBuf,
        /// One line per distinct component with its total quantity.
        #[arg(long)]
        flat: bool,
        /// List at most this many levels below each top-level assembly.
        #[arg(long)]
        depth: Option<usize>,
        /// In the tree, expand repeated sub-assemblies every time. CSV and
        /// JSON always expand them.
        #[arg(long)]
        no_dedupe: bool,
        /// Characters used to draw the tree.
        #[arg(long, value_enum, default_value_t = Charset::Auto)]
        charset: Charset,
        /// How each line of the tree starts.
        #[arg(long, value_enum, default_value_t = Prefix::Indent)]
        prefix: Prefix,
    },
    /// Write one self-contained STEP file per product definition: every
    /// assembly and every part, each with everything that belongs to it.
    ///
    /// Instances are copied byte for byte and renumbered from #1. Aggregates
    /// shared by several products (layers, categories, approvals) keep only
    /// the items of each output. The input must have no dangling references.
    Split {
        /// STEP file to split, or `-` for standard input.
        file: PathBuf,
        /// Directory to write the output files into; created if missing.
        #[arg(short, long, default_value = ".")]
        out: PathBuf,
        /// Overwrite output files that already exist.
        #[arg(long)]
        force: bool,
        /// Also list the entity types that no output contains.
        #[arg(long)]
        report_orphans: bool,
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
        Command::Bom {
            file,
            flat,
            depth,
            no_dedupe,
            charset,
            prefix,
        } => bom(
            file,
            cli.format,
            &BomOptions {
                flat: *flat,
                depth: *depth,
                dedupe: !*no_dedupe,
                charset: *charset,
                prefix: *prefix,
            },
        ),
        Command::Split {
            file,
            out,
            force,
            report_orphans,
        } => split(file, out, cli.format, *force, *report_orphans),
    }
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
        Format::Table | Format::Tree => write_info_table(&mut out, &name, &info, top)?,
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

/// One file written by `stepq split`.
#[derive(serde::Serialize)]
struct SplitFile<'a> {
    file: String,
    kind: &'static str,
    definition: &'a Definition,
    instances: usize,
    pruned: usize,
}

fn split(
    path: &Path,
    dir: &Path,
    format: Format,
    force: bool,
    report_orphans: bool,
) -> anyhow::Result<()> {
    let src = read_input(path)?;
    let name = display_name(path);
    let exchange = parse(&src).with_context(|| format!("parsing {name}"))?;
    let graph = Graph::new(exchange).with_context(|| {
        format!("{name} has a dangling reference, so no output of it could be complete")
    })?;
    let structure = ProductStructure::new(&graph);
    let definitions = structure.definitions();
    if definitions.is_empty() {
        bail!("{name} has no product definitions to split");
    }

    let mut used = std::collections::HashSet::new();
    let mut plans = Vec::with_capacity(definitions.len());
    for (index, definition) in definitions.iter().enumerate() {
        let node = graph
            .node(definition.instance)
            .context("product definition missing from the graph")?;
        let file = output_name(definition, &mut used);
        let target = dir.join(&file);
        if target.exists() && !force {
            bail!(
                "{} already exists; use --force to overwrite",
                target.display()
            );
        }
        plans.push((index, node, file, target));
    }

    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut extractions = Vec::with_capacity(plans.len());
    let mut written = Vec::with_capacity(plans.len());
    for (index, node, file, target) in plans {
        let extraction = stepq::model::extract(&graph, &[node]);
        let output =
            fs::File::create(&target).with_context(|| format!("creating {}", target.display()))?;
        stepq::p21::Writer::new(graph.exchange())
            .numbering(stepq::p21::Numbering::Dense)
            .write_pruned(
                extraction.nodes().iter().copied(),
                extraction.pruned().iter().copied(),
                output,
            )
            .with_context(|| format!("writing {}", target.display()))?;
        written.push(SplitFile {
            file,
            kind: kind(structure.children(index).len() > 0),
            definition: &definitions[index],
            instances: extraction.nodes().len(),
            pruned: extraction.pruned().len(),
        });
        extractions.push(extraction);
    }

    let orphans: Vec<(String, usize)> = if report_orphans {
        orphan_types(&graph, &stepq::model::orphans(&graph, &extractions))
    } else {
        Vec::new()
    };

    write_split_report(
        dir,
        format,
        &written,
        report_orphans.then_some(orphans.as_slice()),
    )
}

/// Prints what `stepq split` wrote, and the orphans when asked for.
fn write_split_report(
    dir: &Path,
    format: Format,
    written: &[SplitFile<'_>],
    orphans: Option<&[(String, usize)]>,
) -> anyhow::Result<()> {
    let mut out = BufWriter::new(io::stdout().lock());
    match format {
        Format::Table | Format::Tree => {
            writeln!(
                out,
                "{:>10}  {:>6}  {:<8}  FILE",
                "INSTANCES", "PRUNED", "TYPE"
            )?;
            for entry in written {
                writeln!(
                    out,
                    "{:>10}  {:>6}  {:<8}  {}",
                    grouped(entry.instances),
                    entry.pruned,
                    entry.kind,
                    entry.file
                )?;
            }
            writeln!(out)?;
            writeln!(out, "wrote {} files to {}", written.len(), dir.display())?;
            if let Some(orphans) = orphans {
                let total: usize = orphans.iter().map(|(_, count)| count).sum();
                writeln!(out, "{} instances are in no output", grouped(total))?;
                for (entity, count) in orphans {
                    writeln!(out, "{:>10}  {entity}", grouped(*count))?;
                }
            }
        }
        Format::Json => {
            #[derive(serde::Serialize)]
            struct Report<'a> {
                directory: String,
                files: &'a [SplitFile<'a>],
                #[serde(skip_serializing_if = "Option::is_none")]
                orphans: Option<&'a [(String, usize)]>,
            }
            serde_json::to_writer_pretty(
                &mut out,
                &Report {
                    directory: dir.display().to_string(),
                    files: written,
                    orphans,
                },
            )?;
            writeln!(out)?;
        }
        Format::Csv => {
            writeln!(
                out,
                "file,type,definition,product_id,product_name,instances,pruned"
            )?;
            for entry in written {
                let product = entry.definition.product.as_ref();
                writeln!(
                    out,
                    "{},{},{},{},{},{},{}",
                    csv_field(&entry.file),
                    entry.kind,
                    entry.definition.instance,
                    csv_field(product.and_then(|p| p.id.as_deref()).unwrap_or_default()),
                    csv_field(product.and_then(|p| p.name.as_deref()).unwrap_or_default()),
                    entry.instances,
                    entry.pruned
                )?;
            }
        }
    }
    out.flush()?;
    Ok(())
}

/// A file name for `definition`'s output: its product id (or name, or
/// definition id) with anything but letters, digits, `.`, `_` and `-`
/// replaced, made unique within `used`.
fn output_name(definition: &Definition, used: &mut std::collections::HashSet<String>) -> String {
    let product = definition.product.as_ref();
    let label = product
        .and_then(|p| p.id.as_deref())
        .or(product.and_then(|p| p.name.as_deref()))
        .or(definition.id.as_deref())
        .unwrap_or_default();
    let mut stem: String = label
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if stem.trim_matches(['_', '.']).is_empty() {
        stem = format!("definition-{}", definition.instance);
    }
    let mut file = format!("{stem}.stp");
    if !used.insert(file.to_ascii_lowercase()) {
        file = format!("{stem}-{}.stp", definition.instance);
        used.insert(file.to_ascii_lowercase());
    }
    file
}

/// Entity types of `nodes`, with counts, most frequent first.
fn orphan_types(graph: &Graph<'_>, nodes: &[usize]) -> Vec<(String, usize)> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for &node in nodes {
        let names: Vec<String> = graph
            .exchange()
            .records(graph.instance(node))
            .map(|r| String::from_utf8_lossy(r.name()).to_ascii_uppercase())
            .collect();
        *counts.entry(names.join("+")).or_default() += 1;
    }
    let mut counts: Vec<(String, usize)> = counts.into_iter().collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
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
            Format::Table | Format::Tree => write_tree(&mut out, structure, usages)?,
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

struct BomOptions {
    flat: bool,
    depth: Option<usize>,
    dedupe: bool,
    charset: Charset,
    prefix: Prefix,
}

fn bom(path: &Path, format: Format, options: &BomOptions) -> anyhow::Result<()> {
    with_structure(path, |structure| {
        // Top-level assemblies; stand-alone parts only if there are none.
        let assemblies: Vec<usize> = structure
            .roots()
            .iter()
            .copied()
            .filter(|&root| structure.children(root).len() > 0)
            .collect();
        let roots = if assemblies.is_empty() {
            structure.roots().to_vec()
        } else {
            assemblies
        };

        let mut out = BufWriter::new(io::stdout().lock());
        if options.flat {
            write_flat_bom(&mut out, structure, &roots, format)?;
        } else {
            match format {
                Format::Table | Format::Tree => {
                    write_bom_tree(&mut out, structure, &roots, options)?;
                }
                Format::Csv => write_bom_csv(&mut out, structure, &roots, options.depth)?,
                Format::Json => {
                    let trees: Vec<JsonBomNode<'_>> = roots
                        .iter()
                        .map(|&root| {
                            let tree =
                                structure.bom_tree(root, BomTreeOptions::new(options.depth, false));
                            json_bom_node(structure, &tree, 0, String::new())
                        })
                        .collect();
                    serde_json::to_writer_pretty(&mut out, &trees)?;
                    writeln!(out)?;
                }
            }
        }
        out.flush()?;
        Ok(())
    })
}

/// Line-drawing characters for a tree.
struct Glyphs {
    tee: &'static str,
    corner: &'static str,
    pipe: &'static str,
    blank: &'static str,
    times: &'static str,
}

impl Glyphs {
    fn new(charset: Charset) -> Self {
        let utf8 = match charset {
            Charset::Utf8 => true,
            Charset::Ascii => false,
            Charset::Auto => locale_is_utf8(),
        };
        if utf8 {
            Self {
                tee: "├── ",
                corner: "└── ",
                pipe: "│   ",
                blank: "    ",
                times: "×",
            }
        } else {
            Self {
                tee: "|-- ",
                corner: "`-- ",
                pipe: "|   ",
                blank: "    ",
                times: "x",
            }
        }
    }
}

/// True if the locale asks for UTF-8, as `LC_ALL`, `LC_CTYPE` or `LANG`
/// (the first one set) says. Windows terminals handle UTF-8.
fn locale_is_utf8() -> bool {
    if cfg!(windows) {
        return true;
    }
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .is_some_and(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("utf-8") || value.contains("utf8")
        })
}

/// Draws a multi-level bill of materials as a tree.
struct BomPrinter<'a> {
    structure: &'a ProductStructure,
    glyphs: Glyphs,
    prefix: Prefix,
    repeated: bool,
}

impl BomPrinter<'_> {
    fn write(
        &mut self,
        out: &mut impl Write,
        node: &BomNode,
        level: usize,
        indent: &str,
        last: bool,
    ) -> io::Result<()> {
        let mut line = definition_label(&self.structure.definitions()[node.definition]);
        // Writing to a String cannot fail.
        if level > 0 && (node.quantity - 1.0).abs() > f64::EPSILON {
            let _ = write!(line, "  {}{}", self.glyphs.times, node.quantity);
        }
        if level > 0 && (node.total - node.quantity).abs() > f64::EPSILON {
            let _ = write!(line, "  ({} total)", node.total);
        }
        if node.repeated {
            line.push_str(" (*)");
            self.repeated = true;
        }
        match self.prefix {
            Prefix::Indent => {
                let branch = match (level, last) {
                    (0, _) => "",
                    (_, true) => self.glyphs.corner,
                    (_, false) => self.glyphs.tee,
                };
                writeln!(out, "{indent}{branch}{line}")?;
            }
            Prefix::Depth => writeln!(out, "{level} {line}")?,
            Prefix::None => writeln!(out, "{line}")?,
        }

        let child_indent = match (level, last) {
            (0, _) => String::new(),
            (_, true) => format!("{indent}{}", self.glyphs.blank),
            (_, false) => format!("{indent}{}", self.glyphs.pipe),
        };
        for (i, component) in node.components.iter().enumerate() {
            let last = i + 1 == node.components.len();
            self.write(out, component, level + 1, &child_indent, last)?;
        }
        Ok(())
    }
}

fn write_bom_tree(
    out: &mut impl Write,
    structure: &ProductStructure,
    roots: &[usize],
    options: &BomOptions,
) -> io::Result<()> {
    let mut printer = BomPrinter {
        structure,
        glyphs: Glyphs::new(options.charset),
        prefix: options.prefix,
        repeated: false,
    };
    for (i, &root) in roots.iter().enumerate() {
        if i > 0 {
            writeln!(out)?;
        }
        let tree = structure.bom_tree(root, BomTreeOptions::new(options.depth, options.dedupe));
        printer.write(out, &tree, 0, "", true)?;

        let lines = structure.bill_of_materials(root);
        if lines.is_empty() {
            writeln!(out, "(no components)")?;
            continue;
        }
        let assemblies = lines.iter().filter(|line| line.is_assembly).count();
        let parts = lines.len() - assemblies;
        let total: f64 = lines
            .iter()
            .filter(|line| !line.is_assembly)
            .map(|line| line.quantity)
            .sum();
        writeln!(out)?;
        writeln!(
            out,
            "{assemblies} {}, {parts} {}, {total} {} in total",
            if assemblies == 1 {
                "sub-assembly"
            } else {
                "sub-assemblies"
            },
            if parts == 1 {
                "distinct part"
            } else {
                "distinct parts"
            },
            if (total - 1.0).abs() < f64::EPSILON {
                "part"
            } else {
                "parts"
            },
        )?;
    }
    if printer.repeated {
        writeln!(
            out,
            "(*) listed in full above; use --no-dedupe to repeat it"
        )?;
    }
    Ok(())
}

/// One row per line of the multi-level BOM, with level and item number.
fn write_bom_csv(
    out: &mut impl Write,
    structure: &ProductStructure,
    roots: &[usize],
    depth: Option<usize>,
) -> io::Result<()> {
    fn rows(
        out: &mut impl Write,
        structure: &ProductStructure,
        node: &BomNode,
        level: usize,
        item: &str,
    ) -> io::Result<()> {
        let definition = &structure.definitions()[node.definition];
        let product = definition.product.as_ref();
        writeln!(
            out,
            "{level},{item},{},{},{},{},{},{}",
            definition.instance,
            csv_field(product.and_then(|p| p.id.as_deref()).unwrap_or_default()),
            csv_field(product.and_then(|p| p.name.as_deref()).unwrap_or_default()),
            kind(node.is_assembly),
            node.quantity,
            node.total,
        )?;
        for (i, component) in node.components.iter().enumerate() {
            let child = if item.is_empty() {
                (i + 1).to_string()
            } else {
                format!("{item}.{}", i + 1)
            };
            rows(out, structure, component, level + 1, &child)?;
        }
        Ok(())
    }

    writeln!(
        out,
        "level,item,definition,product_id,product_name,type,quantity,total_quantity"
    )?;
    for &root in roots {
        let tree = structure.bom_tree(root, BomTreeOptions::new(depth, false));
        rows(out, structure, &tree, 0, "")?;
    }
    Ok(())
}

/// A multi-level BOM node as JSON.
#[derive(serde::Serialize)]
struct JsonBomNode<'a> {
    item: String,
    level: usize,
    quantity: f64,
    total_quantity: f64,
    kind: &'static str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    truncated: bool,
    definition: &'a Definition,
    components: Vec<JsonBomNode<'a>>,
}

fn json_bom_node<'a>(
    structure: &'a ProductStructure,
    node: &BomNode,
    level: usize,
    item: String,
) -> JsonBomNode<'a> {
    JsonBomNode {
        components: node
            .components
            .iter()
            .enumerate()
            .map(|(i, component)| {
                let child = if item.is_empty() {
                    (i + 1).to_string()
                } else {
                    format!("{item}.{}", i + 1)
                };
                json_bom_node(structure, component, level + 1, child)
            })
            .collect(),
        item,
        level,
        quantity: node.quantity,
        total_quantity: node.total,
        kind: kind(node.is_assembly),
        truncated: node.truncated,
        definition: &structure.definitions()[node.definition],
    }
}

/// One line per distinct component under each root, with its total quantity.
fn write_flat_bom(
    out: &mut impl Write,
    structure: &ProductStructure,
    roots: &[usize],
    format: Format,
) -> anyhow::Result<()> {
    let definitions = structure.definitions();
    match format {
        Format::Table | Format::Tree => {
            for (i, &root) in roots.iter().enumerate() {
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
            let boms: Vec<Bom<'_>> = roots
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
            serde_json::to_writer_pretty(&mut *out, &boms)?;
            writeln!(out)?;
        }
        Format::Csv => {
            writeln!(out, "root,product_id,product_name,type,quantity")?;
            for &root in roots {
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
    Ok(())
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
