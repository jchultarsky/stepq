//! Command-line front end for the `stepq` library.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand, ValueEnum};
use stepq::diff::{ChangeKind, Diff};
use stepq::express::Schema;
use stepq::info::Info;
use stepq::lint::{Check, Report};
use stepq::model::{
    BomNode, BomTreeOptions, Definition, Graph, Placement, ProductStructure, Usage,
};
use stepq::p21::{Exchange, Instance, parse};
use stepq::props::{Identifier, Property, PropertyKind, Subject, Value};

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
    /// Check a file for structural problems, without a geometry kernel.
    ///
    /// Always checks syntax, duplicate and dangling references,
    /// transformations whose representations share a context, and
    /// products with no category or a file with no application protocol
    /// definition. With --schema, also checks every record's entity type,
    /// attribute count and list sizes against the schema the file's header
    /// names; tools/fetch-schemas.sh downloads the ISO schemas.
    ///
    /// Exits with status 1 if there are errors. Warnings alone exit 0.
    Lint {
        /// STEP file to check, or `-` for standard input.
        file: PathBuf,
        /// EXPRESS schema file, or a directory of `.exp` files. Repeatable.
        #[arg(long = "schema", value_name = "PATH")]
        schemas: Vec<PathBuf>,
        /// In the table, list every finding instead of the first 10 per check.
        #[arg(long)]
        all: bool,
    },
    /// Show what instances refer to and what refers to them.
    ///
    /// In STEP most links point from the describing entity to the described
    /// one: a product's shape, colours, PMI and placement all refer *to* it,
    /// so "referenced by" is usually where the answers are. With --depth,
    /// references are followed further; each instance is expanded once per
    /// direction and marked (*) where it repeats.
    Refs {
        /// STEP file to inspect, or `-` for standard input.
        file: PathBuf,
        /// Instance names, as `12` or `'#12'` (quote `#` in the shell).
        #[arg(required = true, value_parser = parse_instance_name)]
        ids: Vec<u64>,
        /// Which references to follow.
        #[arg(long, value_enum, default_value_t = Direction::Both)]
        direction: Direction,
        /// How many steps to follow in each direction.
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=64))]
        depth: u8,
        /// Print whole instances instead of their first 100 characters.
        #[arg(long)]
        full: bool,
    },
    /// List instances by entity type or text.
    ///
    /// A complex instance matches a type if any of its partial entities
    /// does. With no filter, lists every instance.
    Query {
        /// STEP file to inspect, or `-` for standard input.
        file: PathBuf,
        /// Entity type, ignoring case. Repeatable: an instance matches if it
        /// has any of them.
        #[arg(long = "type", value_name = "NAME")]
        types: Vec<String>,
        /// Only instances whose text contains this, ignoring ASCII case.
        #[arg(long, value_name = "TEXT")]
        contains: Option<String>,
        /// Print at most this many instances.
        #[arg(long)]
        limit: Option<usize>,
        /// Print how many instances of each entity type match instead.
        #[arg(long)]
        count: bool,
        /// Print whole instances instead of their first 100 characters.
        #[arg(long)]
        full: bool,
    },
    /// List product properties: user-defined attributes, validation
    /// properties and persistent identifiers.
    ///
    /// Properties are grouped under the product definition they belong to,
    /// found by following shapes and shape aspects. Values are printed as
    /// written in the file. With --format csv, one row per value.
    Props {
        /// STEP file to inspect, or `-` for standard input.
        file: PathBuf,
        /// Only list this kind. Repeatable.
        #[arg(long = "kind", value_enum)]
        kinds: Vec<PropsKind>,
    },
    /// Compare two STEP files by what they describe.
    ///
    /// Instance names differ between exports, so the files are compared by
    /// header fields and units, entity-type counts, products (matched by
    /// product id), component quantities per assembly and property values.
    /// Exits with status 1 if the files differ, 0 if they do not.
    Diff {
        /// The first file, or `-` for standard input.
        old: PathBuf,
        /// The second file, or `-` for standard input.
        new: PathBuf,
        /// Only compare this section. Repeatable.
        #[arg(long = "section", value_enum)]
        sections: Vec<DiffSection>,
    },
    /// Remove personal and identifying text before sharing a file.
    ///
    /// Always blanks the header's author, organization and authorization
    /// and every string of PERSON, ORGANIZATION and address entities. With
    /// --anonymize, also renames products to product-1, product-2, … and
    /// usages to usage-1, …, and blanks the header's file name, product and
    /// definition descriptions, usage names, shape names and user-defined
    /// attribute text. Only those strings change: geometry, numbers, entity
    /// types and instance names are copied byte for byte.
    Strip {
        /// STEP file to strip, or `-` for standard input.
        file: PathBuf,
        /// Where to write the result, or `-` for standard output (the report
        /// then goes to standard error).
        #[arg(short, long)]
        out: PathBuf,
        /// Also replace product names, descriptions and attribute text.
        #[arg(long)]
        anonymize: bool,
        /// Overwrite the output file if it exists.
        #[arg(long)]
        force: bool,
    },
}

/// The sections `stepq diff --section` selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum DiffSection {
    /// Header fields, units and the instance count.
    Header,
    /// Instance counts per entity type.
    Types,
    /// Products added, removed or changed.
    Products,
    /// Component quantities per assembly.
    Components,
    /// Property values.
    Properties,
}

/// The kinds `stepq props --kind` selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum PropsKind {
    /// Geometric validation properties.
    Validation,
    /// User-defined attributes.
    User,
    /// Other properties.
    Other,
    /// Persistent identifiers (`id_attribute`).
    Id,
}

impl PropsKind {
    fn of(property: &Property) -> Self {
        match property.kind {
            PropertyKind::Validation => Self::Validation,
            PropertyKind::UserDefined => Self::User,
            _ => Self::Other,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::User => "user",
            Self::Other => "other",
            Self::Id => "id",
        }
    }
}

/// Which references `stepq refs` follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Direction {
    /// Both ways.
    Both,
    /// What the instance refers to.
    Out,
    /// What refers to the instance.
    In,
}

impl Direction {
    fn includes(self, other: Self) -> bool {
        self == Self::Both || self == other
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        // `stepq info big.stp | head` closes the pipe early; that is not an error.
        Err(err) if is_broken_pipe(&err) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> anyhow::Result<ExitCode> {
    let done = match &cli.command {
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
        Command::Lint { file, schemas, all } => return lint(file, schemas, cli.format, *all),
        Command::Refs {
            file,
            ids,
            direction,
            depth,
            full,
        } => refs(
            file,
            ids,
            &RefsOptions {
                direction: *direction,
                depth: usize::from(*depth),
                full: *full,
            },
            cli.format,
        ),
        Command::Query {
            file,
            types,
            contains,
            limit,
            count,
            full,
        } => query(
            file,
            &QueryOptions {
                types,
                contains: contains.as_deref(),
                limit: *limit,
                count: *count,
                full: *full,
            },
            cli.format,
        ),
        Command::Props { file, kinds } => props(file, kinds, cli.format),
        Command::Diff { old, new, sections } => {
            return diff_files(old, new, sections, cli.format);
        }
        Command::Strip {
            file,
            out,
            anonymize,
            force,
        } => strip(file, out, *anonymize, *force, cli.format),
    };
    done.map(|()| ExitCode::SUCCESS)
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

fn strip(
    path: &Path,
    out_path: &Path,
    anonymize: bool,
    force: bool,
    format: Format,
) -> anyhow::Result<()> {
    let src = read_input(path)?;
    let name = display_name(path);
    let exchange = parse(&src).with_context(|| format!("parsing {name}"))?;
    let plan = stepq::strip::strip(&exchange, anonymize);
    let writer = stepq::p21::Writer::new(&exchange).replacements(&plan.replacements);

    let to_stdout = out_path == Path::new("-");
    let output = if to_stdout {
        writer
            .write_all(io::stdout().lock())
            .with_context(|| format!("writing {name}"))?;
        "<stdout>".to_owned()
    } else {
        if out_path.exists() && !force {
            bail!(
                "{} already exists; pass --force to overwrite it",
                out_path.display()
            );
        }
        let file = fs::File::create(out_path)
            .with_context(|| format!("creating {}", out_path.display()))?;
        writer
            .write_all(file)
            .with_context(|| format!("writing {}", out_path.display()))?;
        out_path.display().to_string()
    };

    let strings = plan.replacements.len();
    let report = match format {
        Format::Table | Format::Tree => {
            let plural = |n: usize, word: &str| {
                format!("{} {word}{}", grouped(n), if n == 1 { "" } else { "s" })
            };
            format!(
                "{output}: replaced {} in {}\n",
                plural(strings, "string"),
                plural(plan.instances, "instance")
            )
        }
        Format::Json => {
            let document = serde_json::json!({
                "file": name,
                "output": output,
                "strings": strings,
                "instances": plan.instances,
            });
            format!("{}\n", serde_json::to_string_pretty(&document)?)
        }
        Format::Csv => format!(
            "file,output,strings,instances\n{},{},{strings},{}\n",
            csv_field(&name),
            csv_field(&output),
            plan.instances
        ),
    };
    if to_stdout {
        eprint!("{report}");
    } else {
        print!("{report}");
    }
    Ok(())
}

fn diff_files(
    old_path: &Path,
    new_path: &Path,
    sections: &[DiffSection],
    format: Format,
) -> anyhow::Result<ExitCode> {
    if old_path == Path::new("-") && new_path == Path::new("-") {
        bail!("only one of the two files can be standard input");
    }
    let (old_name, new_name) = (display_name(old_path), display_name(new_path));
    let old_src = read_input(old_path)?;
    let new_src = read_input(new_path)?;
    let old = Graph::build(parse(&old_src).with_context(|| format!("parsing {old_name}"))?);
    let new = Graph::build(parse(&new_src).with_context(|| format!("parsing {new_name}"))?);

    let mut diff = stepq::diff::diff(&old, &new);
    let wants = |section: DiffSection| sections.is_empty() || sections.contains(&section);
    if !wants(DiffSection::Header) {
        diff.header.clear();
    }
    if !wants(DiffSection::Types) {
        diff.entity_types.clear();
    }
    if !wants(DiffSection::Products) {
        diff.products.clear();
    }
    if !wants(DiffSection::Components) {
        diff.components.clear();
    }
    if !wants(DiffSection::Properties) {
        diff.properties.clear();
    }

    let mut out = BufWriter::new(io::stdout().lock());
    match format {
        Format::Table | Format::Tree => write_diff_table(&mut out, &old_name, &new_name, &diff)?,
        Format::Json => {
            #[derive(serde::Serialize)]
            struct Document<'a> {
                old: &'a str,
                new: &'a str,
                differences: usize,
                #[serde(flatten)]
                diff: &'a Diff,
            }
            serde_json::to_writer_pretty(
                &mut out,
                &Document {
                    old: &old_name,
                    new: &new_name,
                    differences: diff.len(),
                    diff: &diff,
                },
            )?;
            writeln!(out)?;
        }
        Format::Csv => write_diff_csv(&mut out, &diff)?,
    }
    out.flush()?;
    Ok(if diff.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn change_mark(kind: ChangeKind) -> char {
    match kind {
        ChangeKind::Added => '+',
        ChangeKind::Removed => '-',
        ChangeKind::Changed => '~',
    }
}

fn change_name(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "added",
        ChangeKind::Removed => "removed",
        ChangeKind::Changed => "changed",
    }
}

fn or_none(value: Option<&String>) -> &str {
    value.map_or("(none)", String::as_str)
}

fn write_diff_table(out: &mut impl Write, old: &str, new: &str, diff: &Diff) -> io::Result<()> {
    writeln!(out, "--- {old}")?;
    writeln!(out, "+++ {new}")?;
    writeln!(out)?;
    if diff.is_empty() {
        return writeln!(out, "no differences");
    }
    if !diff.header.is_empty() {
        writeln!(out, "Header")?;
        for change in &diff.header {
            writeln!(
                out,
                "  {}: {} → {}",
                change.field,
                or_none(change.old.as_ref()),
                or_none(change.new.as_ref())
            )?;
        }
    }
    if !diff.entity_types.is_empty() {
        writeln!(out, "Entity types")?;
        for change in &diff.entity_types {
            let delta = i128::try_from(change.new).unwrap_or(i128::MAX)
                - i128::try_from(change.old).unwrap_or(i128::MAX);
            writeln!(
                out,
                "  {}: {} → {} ({delta:+})",
                change.entity_type,
                grouped(change.old),
                grouped(change.new)
            )?;
        }
    }
    if !diff.products.is_empty() {
        writeln!(out, "Products")?;
        for change in &diff.products {
            let mark = change_mark(change.kind);
            if change.fields.is_empty() {
                writeln!(out, "  {mark} {}", change.product)?;
            }
            for field in &change.fields {
                writeln!(
                    out,
                    "  {mark} {}: {}: {} → {}",
                    change.product,
                    field.field,
                    or_none(field.old.as_ref()),
                    or_none(field.new.as_ref())
                )?;
            }
        }
    }
    if !diff.components.is_empty() {
        writeln!(out, "Components")?;
        for change in &diff.components {
            writeln!(
                out,
                "  {} → {}: {} → {}",
                change.parent, change.child, change.old, change.new
            )?;
        }
    }
    if !diff.properties.is_empty() {
        writeln!(out, "Properties")?;
        for change in &diff.properties {
            let product = if change.product.is_empty() {
                "(no product)"
            } else {
                &change.product
            };
            writeln!(
                out,
                "  {} {product}: {}: {} → {}",
                change_mark(change.kind),
                change.property,
                or_none(change.old.as_ref()),
                or_none(change.new.as_ref())
            )?;
        }
    }
    writeln!(out)?;
    let count = diff.len();
    writeln!(
        out,
        "{} difference{}",
        grouped(count),
        if count == 1 { "" } else { "s" }
    )
}

fn write_diff_csv(out: &mut impl Write, diff: &Diff) -> io::Result<()> {
    writeln!(out, "section,change,subject,field,old,new")?;
    let cell = |value: Option<&String>| csv_field(value.map_or("", String::as_str));
    for change in &diff.header {
        writeln!(
            out,
            "header,changed,,{},{},{}",
            csv_field(&change.field),
            cell(change.old.as_ref()),
            cell(change.new.as_ref())
        )?;
    }
    for change in &diff.entity_types {
        writeln!(
            out,
            "entity_types,changed,{},count,{},{}",
            change.entity_type, change.old, change.new
        )?;
    }
    for change in &diff.products {
        let kind = change_name(change.kind);
        if change.fields.is_empty() {
            writeln!(out, "products,{kind},{},,,", csv_field(&change.product))?;
        }
        for field in &change.fields {
            writeln!(
                out,
                "products,{kind},{},{},{},{}",
                csv_field(&change.product),
                csv_field(&field.field),
                cell(field.old.as_ref()),
                cell(field.new.as_ref())
            )?;
        }
    }
    for change in &diff.components {
        writeln!(
            out,
            "components,changed,{},{},{},{}",
            csv_field(&change.parent),
            csv_field(&change.child),
            change.old,
            change.new
        )?;
    }
    for change in &diff.properties {
        writeln!(
            out,
            "properties,{},{},{},{},{}",
            change_name(change.kind),
            csv_field(&change.product),
            csv_field(&change.property),
            cell(change.old.as_ref()),
            cell(change.new.as_ref())
        )?;
    }
    Ok(())
}

fn props(path: &Path, kinds: &[PropsKind], format: Format) -> anyhow::Result<()> {
    let src = read_input(path)?;
    let name = display_name(path);
    let exchange = parse(&src).with_context(|| format!("parsing {name}"))?;
    let graph = Graph::build(exchange);
    let structure = ProductStructure::new(&graph);
    let found = stepq::props::properties(&graph, &structure);

    let wants = |kind: PropsKind| kinds.is_empty() || kinds.contains(&kind);
    let properties: Vec<&Property> = found
        .properties
        .iter()
        .filter(|property| wants(PropsKind::of(property)))
        .collect();
    let identifiers: Vec<&Identifier> = if wants(PropsKind::Id) {
        found.identifiers.iter().collect()
    } else {
        Vec::new()
    };

    let mut out = BufWriter::new(io::stdout().lock());
    match format {
        Format::Table | Format::Tree => {
            write_props_table(&mut out, &structure, &properties, &identifiers)?;
        }
        Format::Json => {
            #[derive(serde::Serialize)]
            struct Document<'a> {
                file: &'a str,
                definitions: &'a [Definition],
                properties: &'a [&'a Property],
                identifiers: &'a [&'a Identifier],
            }
            serde_json::to_writer_pretty(
                &mut out,
                &Document {
                    file: &name,
                    definitions: structure.definitions(),
                    properties: &properties,
                    identifiers: &identifiers,
                },
            )?;
            writeln!(out)?;
        }
        Format::Csv => write_props_csv(&mut out, &structure, &properties, &identifiers)?,
    }
    out.flush()?;
    Ok(())
}

/// Where a property or identifier is listed: its product definition's
/// position, or after all of them.
fn props_group(structure: &ProductStructure, definition: Option<u64>) -> usize {
    definition
        .and_then(|id| {
            structure
                .definitions()
                .iter()
                .position(|definition| definition.instance == id)
        })
        .unwrap_or(usize::MAX)
}

/// `name = value (MEASURE, unit #id)`, leaving out what is missing or
/// repeats the property's label.
fn value_text(label: &str, value: &Value) -> String {
    let mut text = String::new();
    if let Some(name) = value
        .name
        .as_deref()
        .filter(|n| !n.is_empty() && *n != label)
    {
        let _ = write!(text, " / {name}");
    }
    let _ = write!(text, " = {}", value.value);
    let suffix = match (&value.measure, value.unit) {
        _ if value.value.starts_with('#') => None,
        (Some(measure), Some(unit)) => Some(format!("{measure}, unit #{unit}")),
        (Some(measure), None) => Some(measure.clone()),
        (None, Some(unit)) => Some(format!("unit #{unit}")),
        (None, None) => None,
    };
    if let Some(suffix) = suffix {
        let _ = write!(text, " ({suffix})");
    }
    text
}

fn write_props_table(
    out: &mut impl Write,
    structure: &ProductStructure,
    properties: &[&Property],
    identifiers: &[&Identifier],
) -> io::Result<()> {
    // (group, instance, line), printed in group then file order.
    let mut rows: Vec<(usize, u64, String)> = Vec::new();
    let on = |subject: &Subject| {
        if Some(subject.instance) == subject.product_definition {
            String::new()
        } else {
            format!("  [on {} #{}]", subject.entity, subject.instance)
        }
    };
    let mut value_count = 0;
    for property in properties {
        let group = props_group(structure, property.subject.product_definition);
        let label = property.label();
        let kind = PropsKind::of(property).name();
        if property.values.is_empty() {
            rows.push((
                group,
                property.instance,
                format!("  {kind:<10}  {label}{}", on(&property.subject)),
            ));
        }
        for value in &property.values {
            value_count += 1;
            rows.push((
                group,
                property.instance,
                format!(
                    "  {kind:<10}  {label}{}{}",
                    value_text(label, value),
                    on(&property.subject)
                ),
            ));
        }
    }
    for identifier in identifiers {
        rows.push((
            props_group(structure, identifier.subject.product_definition),
            identifier.instance,
            format!(
                "  {:<10}  {}{}",
                "id",
                identifier.id,
                on(&identifier.subject)
            ),
        ));
    }
    rows.sort_by_key(|(group, instance, _)| (*group, *instance));

    let mut current = None;
    for (group, _, line) in &rows {
        if current != Some(*group) {
            if current.is_some() {
                writeln!(out)?;
            }
            match structure.definitions().get(*group) {
                Some(definition) => writeln!(
                    out,
                    "{}  #{}",
                    definition_label(definition),
                    definition.instance
                )?,
                None => writeln!(out, "(not attached to a product definition)")?,
            }
            current = Some(*group);
        }
        writeln!(out, "{line}")?;
    }
    if !rows.is_empty() {
        writeln!(out)?;
    }
    let plural = |n: usize, one: &str, many: &str| {
        format!("{} {}", grouped(n), if n == 1 { one } else { many })
    };
    writeln!(
        out,
        "{}, {}, {}",
        plural(properties.len(), "property", "properties"),
        plural(value_count, "value", "values"),
        plural(identifiers.len(), "identifier", "identifiers")
    )
}

fn write_props_csv(
    out: &mut impl Write,
    structure: &ProductStructure,
    properties: &[&Property],
    identifiers: &[&Identifier],
) -> io::Result<()> {
    let product = |definition: Option<u64>| {
        definition
            .and_then(|id| structure.definitions().iter().find(|d| d.instance == id))
            .map(definition_label)
            .unwrap_or_default()
    };
    let id = |instance: Option<u64>| instance.map(|id| format!("#{id}")).unwrap_or_default();
    writeln!(
        out,
        "product_definition,product,kind,instance,name,description,subject,subject_type,value_instance,value_name,measure,value,unit"
    )?;
    for property in properties {
        let subject = &property.subject;
        let prefix = format!(
            "{},{},{},#{},{},{},#{},{}",
            id(subject.product_definition),
            csv_field(&product(subject.product_definition)),
            PropsKind::of(property).name(),
            property.instance,
            csv_field(property.name.as_deref().unwrap_or_default()),
            csv_field(property.description.as_deref().unwrap_or_default()),
            subject.instance,
            csv_field(&subject.entity),
        );
        if property.values.is_empty() {
            writeln!(out, "{prefix},,,,,")?;
        }
        for value in &property.values {
            writeln!(
                out,
                "{prefix},#{},{},{},{},{}",
                value.instance,
                csv_field(value.name.as_deref().unwrap_or_default()),
                csv_field(value.measure.as_deref().unwrap_or_default()),
                csv_field(&value.value),
                id(value.unit)
            )?;
        }
    }
    for identifier in identifiers {
        let subject = &identifier.subject;
        writeln!(
            out,
            "{},{},id,#{},{},,#{},{},,,,,",
            id(subject.product_definition),
            csv_field(&product(subject.product_definition)),
            identifier.instance,
            csv_field(&identifier.id),
            subject.instance,
            csv_field(&subject.entity),
        )?;
    }
    Ok(())
}

/// Characters of instance text shown unless `--full` is given.
const TEXT_LIMIT: usize = 100;

/// Accepts `12` or `#12`.
fn parse_instance_name(value: &str) -> Result<u64, String> {
    value
        .strip_prefix('#')
        .unwrap_or(value)
        .parse()
        .map_err(|_| format!("`{value}` is not an instance name such as 12 or #12"))
}

/// The upper-cased entity types of an instance, one per partial entity.
fn entity_types(exchange: &Exchange<'_>, instance: &Instance) -> Vec<String> {
    exchange
        .records(instance)
        .map(|record| String::from_utf8_lossy(record.name()).to_ascii_uppercase())
        .collect()
}

/// An instance's text on one line: whitespace runs collapsed and, unless
/// `full`, cut to [`TEXT_LIMIT`] characters.
fn instance_line(exchange: &Exchange<'_>, instance: &Instance, full: bool) -> String {
    let text = String::from_utf8_lossy(exchange.text(instance));
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if full || line.chars().count() <= TEXT_LIMIT {
        line
    } else {
        let cut: String = line.chars().take(TEXT_LIMIT - 1).collect();
        format!("{cut}…")
    }
}

struct RefsOptions {
    direction: Direction,
    depth: usize,
    full: bool,
}

/// The distinct nodes `node` refers to (`Out`) or that refer to it (`In`),
/// in file order.
fn neighbours(graph: &Graph<'_>, node: usize, direction: Direction) -> Vec<usize> {
    let nodes = if direction == Direction::In {
        graph.referenced_by(node)
    } else {
        graph.references(node)
    };
    let mut distinct: Vec<usize> = nodes.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    distinct
}

fn refs(path: &Path, ids: &[u64], options: &RefsOptions, format: Format) -> anyhow::Result<()> {
    let src = read_input(path)?;
    let name = display_name(path);
    let exchange = parse(&src).with_context(|| format!("parsing {name}"))?;
    let graph = Graph::build(exchange);
    let nodes = ids
        .iter()
        .map(|&id| {
            graph
                .node(id)
                .with_context(|| format!("{name} has no instance #{id}"))
        })
        .collect::<anyhow::Result<Vec<usize>>>()?;
    let directions = [
        ("references", Direction::Out),
        ("referenced by", Direction::In),
    ];
    let directions = directions
        .into_iter()
        .filter(|(_, direction)| options.direction.includes(*direction));

    let mut out = BufWriter::new(io::stdout().lock());
    match format {
        Format::Table | Format::Tree => {
            for (i, &node) in nodes.iter().enumerate() {
                if i > 0 {
                    writeln!(out)?;
                }
                let exchange = graph.exchange();
                writeln!(
                    out,
                    "{}",
                    instance_line(exchange, graph.instance(node), options.full)
                )?;
                for (label, direction) in directions.clone() {
                    writeln!(out, "{label}")?;
                    let next = neighbours(&graph, node, direction);
                    if next.is_empty() {
                        writeln!(out, "  (none)")?;
                    }
                    let mut walk = RefsWalk::new(&graph, direction, options, node);
                    walk.write(&mut out, &next, 1)?;
                }
            }
        }
        Format::Json => {
            let documents: Vec<JsonInstance> = nodes
                .iter()
                .map(|&node| {
                    let mut document = JsonInstance::new(&graph, node);
                    for (_, direction) in directions.clone() {
                        let mut walk = RefsWalk::new(&graph, direction, options, node);
                        let next = neighbours(&graph, node, direction);
                        let linked = Some(walk.json(&next, 1));
                        if direction == Direction::Out {
                            document.references = linked;
                        } else {
                            document.referenced_by = linked;
                        }
                    }
                    document
                })
                .collect();
            serde_json::to_writer_pretty(&mut out, &documents)?;
            writeln!(out)?;
        }
        Format::Csv => {
            writeln!(out, "from,direction,level,instance,type,text")?;
            for &node in &nodes {
                for (_, direction) in directions.clone() {
                    let mut walk = RefsWalk::new(&graph, direction, options, node);
                    let next = neighbours(&graph, node, direction);
                    walk.csv(&mut out, &next, 1)?;
                }
            }
        }
    }
    out.flush()?;
    Ok(())
}

/// One direction of a `stepq refs` walk from one instance.
struct RefsWalk<'g, 'a> {
    graph: &'g Graph<'a>,
    direction: Direction,
    depth: usize,
    full: bool,
    from: u64,
    seen: std::collections::HashSet<usize>,
}

impl<'g, 'a> RefsWalk<'g, 'a> {
    fn new(
        graph: &'g Graph<'a>,
        direction: Direction,
        options: &RefsOptions,
        start: usize,
    ) -> Self {
        Self {
            graph,
            direction,
            depth: options.depth,
            full: options.full,
            from: graph.instance(start).id,
            seen: std::collections::HashSet::from([start]),
        }
    }

    /// Marks `node` seen; true if it was already.
    fn repeated(&mut self, node: usize) -> bool {
        !self.seen.insert(node)
    }

    fn write(&mut self, out: &mut impl Write, nodes: &[usize], level: usize) -> io::Result<()> {
        for &node in nodes {
            let repeated = self.repeated(node);
            let line = instance_line(self.graph.exchange(), self.graph.instance(node), self.full);
            let mark = if repeated { " (*)" } else { "" };
            writeln!(out, "{}{line}{mark}", "  ".repeat(level))?;
            if !repeated && level < self.depth {
                let next = neighbours(self.graph, node, self.direction);
                self.write(out, &next, level + 1)?;
            }
        }
        Ok(())
    }

    fn json(&mut self, nodes: &[usize], level: usize) -> Vec<JsonInstance> {
        nodes
            .iter()
            .map(|&node| {
                let mut document = JsonInstance::new(self.graph, node);
                document.repeated = self.repeated(node);
                if !document.repeated && level < self.depth {
                    let next = neighbours(self.graph, node, self.direction);
                    let linked = Some(self.json(&next, level + 1));
                    if self.direction == Direction::Out {
                        document.references = linked;
                    } else {
                        document.referenced_by = linked;
                    }
                }
                document
            })
            .collect()
    }

    fn csv(&mut self, out: &mut impl Write, nodes: &[usize], level: usize) -> io::Result<()> {
        let direction = if self.direction == Direction::Out {
            "references"
        } else {
            "referenced_by"
        };
        for &node in nodes {
            let repeated = self.repeated(node);
            let exchange = self.graph.exchange();
            let instance = self.graph.instance(node);
            writeln!(
                out,
                "#{},{direction},{level},#{},{},{}",
                self.from,
                instance.id,
                csv_field(&entity_types(exchange, instance).join("+")),
                csv_field(&String::from_utf8_lossy(exchange.text(instance)))
            )?;
            if !repeated && level < self.depth {
                let next = neighbours(self.graph, node, self.direction);
                self.csv(out, &next, level + 1)?;
            }
        }
        Ok(())
    }
}

/// An instance in `stepq refs` and `stepq query` JSON.
#[derive(serde::Serialize)]
struct JsonInstance {
    id: u64,
    types: Vec<String>,
    text: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    repeated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    references: Option<Vec<JsonInstance>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    referenced_by: Option<Vec<JsonInstance>>,
}

impl JsonInstance {
    fn new(graph: &Graph<'_>, node: usize) -> Self {
        Self::of(graph.exchange(), graph.instance(node))
    }

    fn of(exchange: &Exchange<'_>, instance: &Instance) -> Self {
        Self {
            id: instance.id,
            types: entity_types(exchange, instance),
            text: String::from_utf8_lossy(exchange.text(instance)).into_owned(),
            repeated: false,
            references: None,
            referenced_by: None,
        }
    }
}

struct QueryOptions<'a> {
    types: &'a [String],
    contains: Option<&'a str>,
    limit: Option<usize>,
    count: bool,
    full: bool,
}

fn query(path: &Path, options: &QueryOptions<'_>, format: Format) -> anyhow::Result<()> {
    let src = read_input(path)?;
    let name = display_name(path);
    let exchange = parse(&src).with_context(|| format!("parsing {name}"))?;
    let needle = options.contains.map(str::to_ascii_lowercase);
    let matches: Vec<&Instance> = exchange
        .instances()
        .iter()
        .filter(|instance| {
            options.types.is_empty()
                || exchange
                    .records(instance)
                    .any(|record| options.types.iter().any(|name| record.is(name)))
        })
        .filter(|instance| {
            needle.as_ref().is_none_or(|needle| {
                String::from_utf8_lossy(exchange.text(instance))
                    .to_ascii_lowercase()
                    .contains(needle.as_str())
            })
        })
        .collect();

    let mut out = BufWriter::new(io::stdout().lock());
    if options.count {
        write_query_counts(&mut out, &exchange, &matches, options.types, format)?;
        out.flush()?;
        return Ok(());
    }

    let shown = options
        .limit
        .map_or(matches.len(), |limit| limit.min(matches.len()));
    match format {
        Format::Table | Format::Tree => {
            for instance in &matches[..shown] {
                writeln!(out, "{}", instance_line(&exchange, instance, options.full))?;
            }
            if shown < matches.len() {
                writeln!(out, "… {} more (--limit)", grouped(matches.len() - shown))?;
            }
            let plural = if matches.len() == 1 { "" } else { "s" };
            writeln!(out, "{} instance{plural}", grouped(matches.len()))?;
        }
        Format::Json => {
            let documents: Vec<JsonInstance> = matches[..shown]
                .iter()
                .map(|instance| JsonInstance::of(&exchange, instance))
                .collect();
            serde_json::to_writer_pretty(&mut out, &documents)?;
            writeln!(out)?;
        }
        Format::Csv => {
            writeln!(out, "instance,type,text")?;
            for instance in &matches[..shown] {
                writeln!(
                    out,
                    "#{},{},{}",
                    instance.id,
                    csv_field(&entity_types(&exchange, instance).join("+")),
                    csv_field(&String::from_utf8_lossy(exchange.text(instance)))
                )?;
            }
        }
    }
    out.flush()?;
    Ok(())
}

/// Matches per entity type: per requested type if any were given, else per
/// every type the matches have. Most frequent first, then by name.
fn write_query_counts(
    out: &mut impl Write,
    exchange: &Exchange<'_>,
    matches: &[&Instance],
    types: &[String],
    format: Format,
) -> anyhow::Result<()> {
    let mut counts: std::collections::BTreeMap<String, usize> = types
        .iter()
        .map(|name| (name.to_ascii_uppercase(), 0))
        .collect();
    for instance in matches {
        let mut names = entity_types(exchange, instance);
        names.sort();
        names.dedup();
        for name in names {
            if types.is_empty() {
                *counts.entry(name).or_default() += 1;
            } else if let Some(count) = counts.get_mut(&name) {
                *count += 1;
            }
        }
    }
    let mut counts: Vec<(String, usize)> = counts.into_iter().collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    match format {
        Format::Table | Format::Tree => {
            for (name, count) in &counts {
                writeln!(out, "{:>10}  {name}", grouped(*count))?;
            }
        }
        Format::Json => {
            #[derive(serde::Serialize)]
            struct Count<'a> {
                entity_type: &'a str,
                count: usize,
            }
            let rows: Vec<Count<'_>> = counts
                .iter()
                .map(|(name, count)| Count {
                    entity_type: name,
                    count: *count,
                })
                .collect();
            serde_json::to_writer_pretty(&mut *out, &rows)?;
            writeln!(out)?;
        }
        Format::Csv => {
            writeln!(out, "entity_type,count")?;
            for (name, count) in &counts {
                writeln!(out, "{name},{count}")?;
            }
        }
    }
    Ok(())
}

/// Findings of each check listed in the table unless `--all` is given.
const LINT_TABLE_LIMIT: usize = 10;

fn lint(
    path: &Path,
    schema_paths: &[PathBuf],
    format: Format,
    all: bool,
) -> anyhow::Result<ExitCode> {
    let schemas = load_schemas(schema_paths)?;
    let src = read_input(path)?;
    let name = display_name(path);
    let report = stepq::lint::lint(&src, &schemas);

    let mut out = BufWriter::new(io::stdout().lock());
    match format {
        Format::Table | Format::Tree => {
            write_lint_table(&mut out, &name, &report, !schemas.is_empty(), all)?;
        }
        Format::Json => {
            #[derive(serde::Serialize)]
            struct Document<'a> {
                file: &'a str,
                errors: usize,
                warnings: usize,
                #[serde(flatten)]
                report: &'a Report,
            }
            serde_json::to_writer_pretty(
                &mut out,
                &Document {
                    file: &name,
                    errors: report.errors(),
                    warnings: report.warnings(),
                    report: &report,
                },
            )?;
            writeln!(out)?;
        }
        Format::Csv => {
            writeln!(out, "severity,check,instance,message")?;
            for finding in &report.findings {
                writeln!(
                    out,
                    "{},{},{},{}",
                    finding.severity,
                    finding.check,
                    finding
                        .instance
                        .map(|id| format!("#{id}"))
                        .unwrap_or_default(),
                    csv_field(&finding.message)
                )?;
            }
        }
    }
    out.flush()?;
    Ok(if report.errors() > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// Reads every schema named on the command line; a directory contributes
/// its `.exp` files in name order.
fn load_schemas(paths: &[PathBuf]) -> anyhow::Result<Vec<Schema>> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut entries: Vec<PathBuf> = fs::read_dir(path)
                .with_context(|| format!("reading {}", path.display()))?
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|file| {
                    file.extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("exp"))
                })
                .collect();
            if entries.is_empty() {
                bail!("{} contains no .exp schema files", path.display());
            }
            entries.sort();
            files.extend(entries);
        } else {
            files.push(path.clone());
        }
    }
    files
        .iter()
        .map(|file| {
            let src = fs::read(file).with_context(|| format!("reading {}", file.display()))?;
            Schema::parse(&src).with_context(|| format!("reading schema {}", file.display()))
        })
        .collect()
}

fn write_lint_table(
    out: &mut impl Write,
    name: &str,
    report: &Report,
    schemas_given: bool,
    all: bool,
) -> io::Result<()> {
    let schema_line = match (&report.checked_schema, schemas_given) {
        (Some(schema), _) => format!("checked against {schema}"),
        (None, true) => format!(
            "no schema given for {}; schema checks skipped",
            report.file_schemas.join(", ")
        ),
        (None, false) => "schema checks skipped (pass --schema)".to_owned(),
    };
    writeln!(out, "{name}: {schema_line}")?;
    if report.findings.is_empty() {
        writeln!(out, "no problems found")?;
        return Ok(());
    }

    writeln!(out)?;
    writeln!(
        out,
        "{:<8}  {:<28}  {:>10}  MESSAGE",
        "SEVERITY", "CHECK", "INSTANCE"
    )?;
    let mut shown: std::collections::HashMap<Check, usize> = std::collections::HashMap::new();
    let mut hidden: std::collections::BTreeMap<Check, usize> = std::collections::BTreeMap::new();
    for finding in &report.findings {
        let count = shown.entry(finding.check).or_default();
        if !all && *count >= LINT_TABLE_LIMIT {
            *hidden.entry(finding.check).or_default() += 1;
            continue;
        }
        *count += 1;
        writeln!(
            out,
            "{:<8}  {:<28}  {:>10}  {}",
            finding.severity,
            finding.check,
            finding
                .instance
                .map(|id| format!("#{id}"))
                .unwrap_or_default(),
            finding.message
        )?;
    }
    for (check, count) in &hidden {
        writeln!(
            out,
            "{:<8}  {:<28}  {:>10}  … {} more (--all lists them)",
            check.severity(),
            check,
            "",
            grouped(*count)
        )?;
    }

    let plural =
        |n: usize, word: &str| format!("{} {word}{}", grouped(n), if n == 1 { "" } else { "s" });
    writeln!(out)?;
    writeln!(
        out,
        "{}, {}",
        plural(report.errors(), "error"),
        plural(report.warnings(), "warning")
    )?;
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
