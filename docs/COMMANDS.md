# Command reference

Every `stepq` command, its options and real output. The examples use a
small AP214 assembly (`assembly.stp`: a plate, two L-bracket
sub-assemblies with three nut-and-bolt assemblies each, and a rod
assembly), an AP242 part with semantic PMI (`pmi-part.stp`) and a
synthetic part with two solids (`part.stp`). Long outputs are cut where
a line reads `…`; nothing else is edited.

`stepq help <command>` prints the same options with their full help text.

- [Common options](#common-options)
- [info](#info) · [tree](#tree) · [bom](#bom) · [split](#split) ·
  [assemble](#assemble) · [lint](#lint) · [refs](#refs) · [query](#query) ·
  [props](#props) · [diff](#diff) · [strip](#strip) · [pmi](#pmi)

## Common options

`--format table|json|csv|tree` is accepted by every command, before or
after the command name. The default is `table`.

| Format | Output |
|---|---|
| `table` | Human-readable text. |
| `json` | One pretty-printed JSON document. |
| `csv` | Comma-separated values with a header row; fields holding a comma, quote or line break are quoted. |
| `tree` | The multi-level view of commands that have one (`bom`, where it is the same as `table`). Other commands print their table. |

Commands that write files (`split`, `assemble`, `strip`) use `--format`
for the report of what they wrote.

**Standard input.** Every input file argument accepts `-` to read
standard input. `diff` accepts it for at most one of its two files, and
`assemble -` resolves the files a master refers to relative to the
current directory. `strip` and `assemble` also accept `-o -` to write the
result to standard output; their report then goes to standard error.

**Exit codes.**

| Code | Meaning |
|---|---|
| 0 | Success. Also when the output pipe is closed early (`stepq info big.stp \| head`). |
| 1 | An error, printed as `error: …` on standard error (unreadable or unparsable file, output file already exists, …). Also `lint` when it finds errors, and `diff` when the files differ. |
| 2 | Invalid command line (unknown command or option, missing argument). |

## info

Summarises a file: header fields, schema, declared units, instance and
complex-instance counts, data sections, products and assembly usages,
references to undefined instances, and a histogram of entity types. It is
the quick first look at a file you have never seen.

```
stepq info [--top N] FILE
```

| Option | |
|---|---|
| `FILE` | STEP file to inspect, or `-` for standard input. |
| `--top N` | How many entity types to list in the table; `0` lists all. Default 20. |

With `--format csv`, only the entity-type histogram is printed. With
`--format json`, the whole summary, with every entity type.

Library: [`stepq::info::Info::new`](https://docs.rs/stepq/latest/stepq/info/struct.Info.html).

```console
$ stepq info assembly.stp --top 5
File                  assembly.stp (83,418 bytes)
Schema                AUTOMOTIVE_DESIGN { 1 2 10303 214 0 1 1 1 }
Name                  assembly.stp
Time stamp            1999-09-09T14:22:00
Originating system    AutoCAD 2000
Preprocessor          AutoCAD STEP 2000
Authorization         , , 
Implementation level  2;1
Units                 length millimetre; plane angle radian, degree; solid angle steradian
Instances             1,855 (26 complex) in 1 data section
Products              9 (13 assembly usages)

Entity types (64)
       316  DIRECTION
       277  CARTESIAN_POINT
       252  ORIENTED_EDGE
       126  EDGE_CURVE
       123  AXIS2_PLACEMENT_3D
            … 59 more (--top 0 lists all)
```

```console
$ stepq info assembly.stp --format json
{
  "file": "assembly.stp",
  "bytes": 83418,
  "header": {
    "description": [],
    "implementation_level": "2;1",
    "name": "assembly.stp",
    "time_stamp": "1999-09-09T14:22:00",
    "author": [],
    "organization": [],
    "preprocessor_version": "AutoCAD STEP 2000",
    "originating_system": "AutoCAD 2000",
    "authorization": ", , ",
    "schemas": [
      "AUTOMOTIVE_DESIGN { 1 2 10303 214 0 1 1 1 }"
    ]
  },
  "sections": 1,
  "instances": 1855,
  "complex_instances": 26,
  "products": 9,
  "assembly_usages": 13,
  "unresolved_references": 0,
  …
```

```console
$ stepq info assembly.stp --format csv
entity_type,count
DIRECTION,316
CARTESIAN_POINT,277
ORIENTED_EDGE,252
…
```

## tree

Prints the assembly hierarchy: every top-level product definition and
the components under it. A component used several times by the same
assembly is shown once with a count. With `--usages`, every
`next_assembly_usage_occurrence` is listed separately with the entities
that place it — the context-dependent shape representation, the
representation relationship and its transformation, or the mapped item.

```
stepq tree [--usages] FILE
```

| Option | |
|---|---|
| `FILE` | STEP file to inspect, or `-` for standard input. |
| `--usages` | List every usage separately, with the entities that place it. |

With `--format json`, every definition and usage of the product
structure; with `--format csv`, one row per usage.

Library: [`stepq::model::ProductStructure`](https://docs.rs/stepq/latest/stepq/model/struct.ProductStructure.html)
(`roots`, `definitions`, `usages`, `components`).

```console
$ stepq tree assembly.stp
ASSEMBLY
├── PLATE
├── L-BRACKET ASSEMBLY  ×2
│   ├── L-BRACKET
│   └── NUT-BOLT ASSEMBLY  ×3
│       ├── BOLT
│       └── NUT
└── ROD-ASSEMBLY
    ├── ROD
    └── NUT  ×2

9 product definitions, 13 assembly usages, 1 top-level
```

```console
$ stepq tree assembly.stp --usages
ASSEMBLY
├── PLATE  #691 'PLATE_1'  placed by #693 → #690 (#686)
├── L-BRACKET ASSEMBLY  #1764 'L-BRACKET ASSEMBLY_1'  placed by #1766 → #1763 (#1759)
│   ├── L-BRACKET  #1247 'L-BRACKET_1'  placed by #1249 → #1246 (#1242)
│   ├── NUT-BOLT ASSEMBLY  #1728 'NUT-BOLT ASSEMBLY_1'  placed by #1730 → #1727 (#1723)
│   │   ├── BOLT  #1459 'BOLT_1'  placed by #1461 → #1458 (#1454)
…
```

```console
$ stepq tree assembly.stp --format csv
usage,usage_id,parent,parent_product,child,child_product,quantity,reference_designator,placement_kind,placement,transformation,reversed
691,NAUO1,52,ASSEMBLY,72,PLATE,1,,shape_relationship,690,686,false
1247,NAUO2,701,L-BRACKET ASSEMBLY,712,L-BRACKET,1,,shape_relationship,1246,1242,false
1459,NAUO3,1257,NUT-BOLT ASSEMBLY,1268,BOLT,1,,shape_relationship,1458,1454,false
…
```

## bom

Emits the bill of materials of each top-level assembly (of each
stand-alone part, if the file has no assembly). By default it is a
multi-level BOM: every component under its assembly with its quantity per
assembly and, where different, its total quantity. Quantities come from
the number of usages and from `quantified_assembly_component_usage`. With
`--flat`, one line per distinct component with its total quantity.

```
stepq bom [--flat] [--depth N] [--no-dedupe] [--charset auto|utf8|ascii] [--prefix indent|depth|none] FILE
```

| Option | |
|---|---|
| `FILE` | STEP file to inspect, or `-` for standard input. |
| `--flat` | One line per distinct component with its total quantity. |
| `--depth N` | List at most this many levels below each top-level assembly. The summary line still counts the whole BOM, and says so when levels are left out. |
| `--no-dedupe` | In the tree, expand repeated sub-assemblies every time. By default a repeated sub-assembly is expanded once and marked `(*)`. CSV and JSON always expand them. |
| `--charset auto\|utf8\|ascii` | Characters used to draw the tree. `auto` (the default) uses box-drawing characters when `LC_ALL`, `LC_CTYPE` or `LANG` asks for UTF-8, ASCII otherwise. |
| `--prefix indent\|depth\|none` | How each tree line starts: tree lines (default), the level number (0 for the top assembly), or nothing. |

As a table or tree the BOM is drawn as a tree; as CSV, one row per line
with its level and item number (`2.2.1`); as JSON, nested.

Library: [`ProductStructure::bom_tree`](https://docs.rs/stepq/latest/stepq/model/struct.ProductStructure.html#method.bom_tree)
(multi-level) and `ProductStructure::bill_of_materials` (flat).

```console
$ stepq bom assembly.stp
ASSEMBLY
├── PLATE
├── L-BRACKET ASSEMBLY  ×2
│   ├── L-BRACKET  (2 total)
│   └── NUT-BOLT ASSEMBLY  ×3  (6 total)
│       ├── BOLT  (6 total)
│       └── NUT  (6 total)
└── ROD-ASSEMBLY
    ├── ROD
    └── NUT  ×2

3 sub-assemblies, 5 distinct parts, 18 parts in total
```

```console
$ stepq bom assembly.stp --format csv
level,item,definition,product_id,product_name,type,quantity,total_quantity
0,,52,ASSEMBLY,ASSEMBLY,assembly,1,1
1,1,72,PLATE,PLATE,part,1,1
1,2,701,L-BRACKET ASSEMBLY,L-BRACKET ASSEMBLY,assembly,2,2
2,2.1,712,L-BRACKET,L-BRACKET,part,1,2
2,2.2,1257,NUT-BOLT ASSEMBLY,NUT-BOLT ASSEMBLY,assembly,3,6
3,2.2.1,1268,BOLT,BOLT,part,1,6
3,2.2.2,1469,NUT,NUT,part,1,6
1,3,1786,ROD-ASSEMBLY,ROD-ASSEMBLY,assembly,1,1
2,3.1,1797,ROD,ROD,part,1,1
2,3.2,1469,NUT,NUT,part,2,2
```

```console
$ stepq bom assembly.stp --flat
ASSEMBLY
  QUANTITY  TYPE      PRODUCT
         1  part      PLATE
         2  assembly  L-BRACKET ASSEMBLY
         2  part      L-BRACKET
         6  assembly  NUT-BOLT ASSEMBLY
         6  part      BOLT
         8  part      NUT
         1  assembly  ROD-ASSEMBLY
         1  part      ROD
```

```console
$ stepq bom assembly.stp --depth 1 --charset ascii --prefix depth
0 ASSEMBLY
1 PLATE
1 L-BRACKET ASSEMBLY  x2
1 ROD-ASSEMBLY

3 sub-assemblies, 5 distinct parts, 18 parts in total, including levels below --depth 1
```

## split

Writes one self-contained STEP file per product definition: every
assembly and every part, each with everything that belongs to it —
shapes, placements, colours, layers, properties, PMI. Instances are
copied byte for byte and renumbered from `#1`; aggregates shared by
several products (layers, categories, approvals) keep only the items of
each output. A file is named after its product id (or name), with
characters other than letters, digits, `.`, `_` and `-` replaced by `_`.
The input must have no dangling references. No geometry is evaluated: see
[ARCHITECTURE.md](ARCHITECTURE.md#the-back-reference-problem) for how the
closure is computed.

```
stepq split [-o DIR] [--force] [--report-orphans] [--bodies] [--master] FILE
```

| Option | |
|---|---|
| `FILE` | STEP file to split, or `-` for standard input. |
| `-o`, `--out DIR` | Directory to write the output files into; created if missing. Default `.`. |
| `--force` | Overwrite output files that already exist. Without it, `split` stops before writing anything if a file it would write exists, `--bodies` files included. |
| `--report-orphans` | Also list the entity types, with counts, of the instances that no output contains. |
| `--bodies` | Also write one file per solid of every part with several, `<part>.body-<n>.stp`: the part with its other solids, and everything only they bring (faces, colours), left out. |
| `--master` | Write assemblies as master files that refer to their components' files instead of copying their geometry. Parts are written as usual. |

The report lists each file with its instance count, how many of its
instances had shared lists pruned, and its type (`assembly`, `part`,
`body` or `master`). With `--format csv`, one row per file with its
product definition, product id and name; with `--format json`, the files
with their full definitions and, with `--report-orphans`, the orphans.

Library: [`stepq::model::extract`](https://docs.rs/stepq/latest/stepq/model/fn.extract.html)
computes an output's instances, `model::extract_excluding` one with some
instances left out (`--bodies`, `--master`), `model::orphans` what no
extraction holds, and `p21::Writer::write_pruned` writes a selection.
Master files are written by the CLI.

```console
$ stepq split assembly.stp --out parts --report-orphans
 INSTANCES  PRUNED  TYPE      FILE
     1,854       7  assembly  ASSEMBLY.stp
       650       7  part      PLATE.stp
     1,062       7  assembly  L-BRACKET_ASSEMBLY.stp
       568       7  part      L-BRACKET.stp
       490       7  assembly  NUT-BOLT_ASSEMBLY.stp
       224       7  part      BOLT.stp
       280       7  part      NUT.stp
       419       7  assembly  ROD-ASSEMBLY.stp
       144       7  part      ROD.stp

wrote 9 files to parts
1 instance is in no output
         1  PRODUCT_CATEGORY
```

### --bodies

A part whose shape representation holds several `manifold_solid_brep`s
gets one extra file per solid. Here `part.stp` is a part with two solids,
a colour on the second and a layer holding both; `P.body-2.stp` keeps the
colour, and its layer lists only its own solid.

```console
$ stepq split part.stp --bodies --out bodies --format csv
file,type,definition,product_id,product_name,instances,pruned
P.stp,part,10,P,plate,24,1
P.body-1.stp,body,10,P,plate,18,2
P.body-2.stp,body,10,P,plate,20,2
```

A solid referenced other than as a list item cannot be separated; the
write then fails rather than produce a dangling file.

### --master

Every assembly is written as a master file following the CAx-IF
Recommended Practices for External References: each component keeps its
product, definition and placements and a shape with no geometry, and
refers to its own file through `document_file`,
`applied_external_identification_assignment` and
`applied_document_reference`. Open CASCADE reads a top master with its
nested masters as the whole assembly. [`assemble`](#assemble) is the
inverse.

```console
$ stepq split assembly.stp --master --out masters
 INSTANCES  PRUNED  TYPE      FILE
       161       9  master    ASSEMBLY.stp
       650       7  part      PLATE.stp
       123       9  master    L-BRACKET_ASSEMBLY.stp
       568       7  part      L-BRACKET.stp
        97       7  master    NUT-BOLT_ASSEMBLY.stp
       224       7  part      BOLT.stp
       280       7  part      NUT.stp
       106       8  master    ROD-ASSEMBLY.stp
       144       7  part      ROD.stp

wrote 9 files to masters
$ grep 'PLATE.stp' masters/ASSEMBLY.stp
#1948=DOCUMENT_FILE('PLATE.stp','',$,#1947,'',$);
#1952=APPLIED_EXTERNAL_IDENTIFICATION_ASSIGNMENT('PLATE.stp',#1950,#1951,(#1948));
```

## assemble

Merges a master file and the component files it refers to into one file:
the inverse of `split --master`. Every component stub with a CAx-IF
external reference (`document_file`, `applied_document_reference`) is
replaced by the instances of the file it names, read relative to the
master's folder, and the reference entities are dropped. Component files
that are masters themselves are merged first; a file named by several
masters is merged once. Instances are copied as written and renamed after
the master's.

```
stepq assemble -o OUT [--force] FILE
```

| Option | |
|---|---|
| `FILE` | Master STEP file, or `-` for standard input (references are then read relative to the current directory). |
| `-o`, `--out OUT` | Where to write the result, or `-` for standard output (the report then goes to standard error). Required. |
| `--force` | Overwrite the output file if it exists. |

The report lists the merged files; as CSV, one row per merged file.

Library: [`stepq::assemble::assemble`](https://docs.rs/stepq/latest/stepq/assemble/fn.assemble.html),
which takes a loader closure for the referenced files.

```console
$ stepq assemble masters/ASSEMBLY.stp -o whole.stp
whole.stp: merged 8 files
  PLATE.stp
  L-BRACKET_ASSEMBLY.stp
  L-BRACKET.stp
  NUT-BOLT_ASSEMBLY.stp
  BOLT.stp
  NUT.stp
  ROD-ASSEMBLY.stp
  ROD.stp
$ stepq diff assembly.stp whole.stp --section products --section components
--- assembly.stp
+++ whole.stp

no differences
```

```console
$ stepq assemble masters/ROD-ASSEMBLY.stp -o - --format csv > rod.stp
file,output,merged
masters/ROD-ASSEMBLY.stp,<stdout>,NUT.stp
masters/ROD-ASSEMBLY.stp,<stdout>,ROD.stp
```

## lint

Checks a file for structural problems without a geometry kernel. It
always checks syntax, duplicate instance names, dangling references,
transformations whose two representations share a context, a missing
`application_protocol_definition` and products in no
`product_related_product_category`. With `--schema`, it also checks every
record's entity type, attribute count and list sizes against the schema
the file's header names. Schemas are not shipped (ISO copyright);
`tools/fetch-schemas.sh` downloads them.

```
stepq lint [--schema PATH]... [--all] FILE
```

| Option | |
|---|---|
| `FILE` | STEP file to check, or `-` for standard input. |
| `--schema PATH` | EXPRESS schema file, or a directory of `.exp` files. Repeatable. |
| `--all` | In the table, list every finding instead of the first 10 per check. |

| Check | Severity | Needs `--schema` |
|---|---|---|
| `syntax` | error | |
| `duplicate-id` | error | |
| `dangling-reference` | error | |
| `unknown-entity` | error | yes |
| `attribute-count` | error | yes |
| `too-few-elements` | error | yes |
| `shared-context` | error | |
| `missing-application-protocol` | warning | |
| `uncategorized-product` | warning | |

Exits with status 1 if there are errors; warnings alone exit 0.

Library: [`stepq::lint::lint`](https://docs.rs/stepq/latest/stepq/lint/fn.lint.html).

```console
$ stepq lint assembly.stp
assembly.stp: schema checks skipped (pass --schema)
no problems found
```

```console
$ stepq lint part.stp
part.stp: schema checks skipped (pass --schema)

SEVERITY  CHECK                           INSTANCE  MESSAGE
warning   uncategorized-product                #11  PRODUCT is in no PRODUCT_RELATED_PRODUCT_CATEGORY

0 errors, 1 warning
```

The same assembly with one `product_definition_shape` removed, read from
standard input:

```console
$ grep -v '^#1471=' assembly.stp | stepq lint - --format json
{
  "file": "<stdin>",
  "errors": 1,
  "warnings": 0,
  "file_schemas": [
    "AUTOMOTIVE_DESIGN { 1 2 10303 214 0 1 1 1 }"
  ],
  "checked_schema": null,
  "findings": [
    {
      "severity": "error",
      "check": "dangling-reference",
      "instance": 1472,
      "message": "references undefined instance #1471"
    }
  ]
}
$ echo $?
1
```

## refs

Shows what instances refer to and what refers to them. In STEP most links
point from the describing entity to the described one — a product's
shape, colours, PMI and placement all refer *to* it — so "referenced by"
is usually where the answers are. With `--depth`, references are followed
further; each instance is expanded once per direction and marked `(*)`
where it repeats.

```
stepq refs [--direction both|out|in] [--depth N] [--full] FILE ID...
```

| Option | |
|---|---|
| `FILE` | STEP file to inspect, or `-` for standard input. |
| `ID...` | One or more instance names, as `12` or `'#12'` (quote `#` in the shell). |
| `--direction both\|out\|in` | Which references to follow: both (default), what the instance refers to, or what refers to it. |
| `--depth N` | How many steps to follow in each direction, 1 to 64. Default 1. |
| `--full` | Print whole instances instead of their first 100 characters. |

With `--format json`, one nested document per instance; with
`--format csv`, one row per linked instance with its level.

Library: [`stepq::model::Graph`](https://docs.rs/stepq/latest/stepq/model/struct.Graph.html)
(`references`, `referenced_by`).

```console
$ stepq refs assembly.stp 1469
#1469=PRODUCT_DEFINITION('None','',#1468,#51);
references
  #51=PRODUCT_DEFINITION_CONTEXT('part definition',#29,'design');
  #1468=PRODUCT_DEFINITION_FORMATION_WITH_SPECIFIED_SOURCE('None','',#1467,.BOUGHT.);
referenced by
  #57=APPLIED_PERSON_AND_ORGANIZATION_ASSIGNMENT(#55,#56,(#52,#72,#701,#712,#1257,#1268,#1469,#1786,#…
  #1471=PRODUCT_DEFINITION_SHAPE('PDS7','',#1469);
  #1716=NEXT_ASSEMBLY_USAGE_OCCURRENCE('NAUO4','NUT_1','NUT_1',#1257,#1469,$);
  #1920=NEXT_ASSEMBLY_USAGE_OCCURRENCE('NAUO11','NUT_2','NUT_2',#1786,#1469,$);
  #1932=NEXT_ASSEMBLY_USAGE_OCCURRENCE('NAUO12','NUT_3','NUT_3',#1786,#1469,$);
```

```console
$ stepq refs assembly.stp 1469 --direction out --format json
[
  {
    "id": 1469,
    "types": [
      "PRODUCT_DEFINITION"
    ],
    "text": "#1469=PRODUCT_DEFINITION('None','',#1468,#51);",
    "references": [
      {
        "id": 51,
        "types": [
          "PRODUCT_DEFINITION_CONTEXT"
        ],
        "text": "#51=PRODUCT_DEFINITION_CONTEXT('part definition',#29,'design');"
      },
      {
        "id": 1468,
        "types": [
          "PRODUCT_DEFINITION_FORMATION_WITH_SPECIFIED_SOURCE"
        ],
        "text": "#1468=PRODUCT_DEFINITION_FORMATION_WITH_SPECIFIED_SOURCE('None','',#1467,.BOUGHT.);"
      }
    ]
  }
]
```

## query

Lists instances by entity type, by text, or both. Types are matched
ignoring case, and a complex instance matches a type if any of its
partial entities does. With no filter, every instance is listed.

```
stepq query [--type NAME]... [--contains TEXT] [--limit N] [--count] [--full] FILE
```

| Option | |
|---|---|
| `FILE` | STEP file to inspect, or `-` for standard input. |
| `--type NAME` | Entity type, ignoring case. Repeatable: an instance matches if it has any of them. |
| `--contains TEXT` | Only instances whose text contains this, ignoring ASCII case. |
| `--limit N` | Print at most this many instances. |
| `--count` | Print how many instances of each entity type match instead. |
| `--full` | Print whole instances instead of their first 100 characters. |

With `--format json`, an array of `{id, types, text}`; with
`--format csv`, one row per instance. `--count` prints `entity_type,count`
rows as CSV.

Library: [`p21::Exchange::instances_of`](https://docs.rs/stepq/latest/stepq/p21/struct.Exchange.html#method.instances_of)
finds instances by type.

```console
$ stepq query assembly.stp --type product --limit 3
#32=PRODUCT('ASSEMBLY','ASSEMBLY','',(#31));
#70=PRODUCT('PLATE','PLATE','',(#31));
#699=PRODUCT('L-BRACKET ASSEMBLY','L-BRACKET ASSEMBLY','',(#31));
… 6 more (--limit)
9 instances
```

```console
$ stepq query assembly.stp --type product_definition --type next_assembly_usage_occurrence --count
        13  NEXT_ASSEMBLY_USAGE_OCCURRENCE
         9  PRODUCT_DEFINITION
```

```console
$ stepq query assembly.stp --contains "'NUT'" --format csv
instance,type,text
#1467,PRODUCT,"#1467=PRODUCT('NUT','NUT','',(#31));"
```

## props

Lists product properties — user-defined attributes, validation properties
(geometric: volume, area, centroid; and attribute counts), other
properties — and persistent identifiers (`id_attribute`). They are
grouped under the product definition they belong to, found by following
shapes and shape aspects. Values are printed as written in the file;
nothing is recomputed. In the table, a value whose text spans lines is
printed on one line, and a property with no name is shown as `(unnamed)`.

```
stepq props [--kind validation|user|other|id]... FILE
```

| Option | |
|---|---|
| `FILE` | STEP file to inspect, or `-` for standard input. |
| `--kind validation\|user\|other\|id` | Only list this kind. Repeatable. |

With `--format csv`, one row per value; with `--format json`, the product
definitions, properties and identifiers (where the property kind is
written `validation`, `user` or `other`, as `--kind` names them).

Library: [`stepq::props::properties`](https://docs.rs/stepq/latest/stepq/props/fn.properties.html).

```console
$ stepq props pmi-part.stp --kind user --kind validation
BRACKET  #4368
  user        Modeled By = Engineer
  user        CAGE Code = 64JW1
  user        Company = ACME
  validation  attribute validation property / part user attributes = 3.
  validation  attribute validation property / text user attributes = 3.
  validation  attribute validation property / integer user attributes = 0.
  validation  attribute validation property / real user attributes = 0.
  validation  attribute validation property / boolean user attributes = 0.
  validation  volume of BRACKET / volume measure = 14644822.6361138 (VOLUME_MEASURE, unit #639)  [on PRODUCT_DEFINITION_SHAPE #4269]
  validation  volume of BRACKET / wetted area measure = 807080.802199914 (AREA_MEASURE, unit #640)  [on PRODUCT_DEFINITION_SHAPE #4269]
  validation  volume of BRACKET / centre point = #3942=CARTESIAN_POINT('centre point',(-2.29264395139875,-1.36345168588643,-32.2974419857648));  [on PRODUCT_DEFINITION_SHAPE #4269]
  user        (unnamed) = B  [on SHAPE_ASPECT #316]
  user        (unnamed) = A  [on SHAPE_ASPECT #317]

7 properties, 13 values, 0 identifiers
```

```console
$ stepq props pmi-part.stp --kind user --format csv
product_definition,product,kind,instance,name,description,subject,subject_type,value_instance,value_name,measure,value,unit
#4368,BRACKET,user,#4331,Modeled By,,#4368,PRODUCT_DEFINITION,#4346,,,Engineer,
#4368,BRACKET,user,#4332,CAGE Code,,#4368,PRODUCT_DEFINITION,#4347,,,64JW1,
#4368,BRACKET,user,#4333,Company,,#4368,PRODUCT_DEFINITION,#4348,,,ACME,
#4368,BRACKET,user,#4340,,,#316,SHAPE_ASPECT,#4349,,,B,
#4368,BRACKET,user,#4341,,,#317,SHAPE_ASPECT,#4350,,,A,
```

## diff

Compares two STEP files by what they describe. Instance names differ
between exports, so the files are compared by header fields, units and
instance count, entity-type counts, products (matched by product id),
component quantities per assembly and property values (with `#id`s in
labels ignored). In the table, `-` marks something removed, `+` added and
`~` changed.

```
stepq diff [--section header|types|products|components|properties]... OLD NEW
```

| Option | |
|---|---|
| `OLD`, `NEW` | The two files; one of them may be `-` for standard input. |
| `--section NAME` | Only compare this section. Repeatable. `header` is header fields, units and the instance count; `types` instance counts per entity type; `products` products added, removed or changed; `components` component quantities per assembly; `properties` property values. |

Exits with status 1 if the files differ, 0 if they do not. With
`--format csv`, one row per change as `section,change,subject,field,old,new`.

Library: [`stepq::diff::diff`](https://docs.rs/stepq/latest/stepq/diff/fn.diff.html).

Here `new.stp` is `old.stp` after `strip --anonymize`:

```console
$ stepq diff old.stp new.stp
--- old.stp
+++ new.stp

Header
  authorization: , ,  → (none)
  name: assembly.stp → (none)
Products
  - ASSEMBLY
  - BOLT
…
  + product-8
  + product-9
Components
  ASSEMBLY → L-BRACKET ASSEMBLY: 2 → 0
  ASSEMBLY → PLATE: 1 → 0
…
  product-8 → product-7: 0 → 2
  product-8 → product-9: 0 → 1

38 differences
$ echo $?
1
```

```console
$ stepq diff old.stp new.stp --section header --section products --format csv
section,change,subject,field,old,new
header,changed,,authorization,", , ",
header,changed,,name,assembly.stp,
products,removed,ASSEMBLY,,,
products,removed,BOLT,,,
…
products,added,product-8,,,
products,added,product-9,,,
```

## strip

Removes personal and identifying text before sharing a file. It always
blanks the header's author, organization and authorization and every
string of `person`, `organization` and address entities. With
`--anonymize`, it also renames products to `product-1`, `product-2`, …
and usages to `usage-1`, …, and blanks the header's file name, product
and definition descriptions, usage names, shape names and user-defined
attribute text. Only those strings change: geometry, numbers, entity
types and instance names are copied byte for byte.

```
stepq strip -o OUT [--anonymize] [--force] FILE
```

| Option | |
|---|---|
| `FILE` | STEP file to strip, or `-` for standard input. |
| `-o`, `--out OUT` | Where to write the result, or `-` for standard output (the report then goes to standard error). Required. |
| `--anonymize` | Also replace product names, descriptions and attribute text. |
| `--force` | Overwrite the output file if it exists. |

The report says how many strings were replaced in how many instances.

Library: [`stepq::strip::strip`](https://docs.rs/stepq/latest/stepq/strip/fn.strip.html)
returns the replacements, which `p21::Writer::replacements` applies.

```console
$ stepq strip assembly.stp -o stripped.stp
stripped.stp: replaced 21 strings in 8 instances
$ diff assembly.stp stripped.stp
4c4
< FILE_NAME('assembly.stp','1999-09-09T14:22:00',(''),(''),'AutoCAD STEP 2000','AutoCAD 2000',', , ');
---
> FILE_NAME('assembly.stp','1999-09-09T14:22:00',(''),(''),'AutoCAD STEP 2000','AutoCAD 2000','');
22,23c22,23
< #35=PERSON('1','Design','Joe',$,$,$);
< #36=ORGANIZATION($,'None','None');
---
> #35=PERSON('','','',$,$,$);
> #36=ORGANIZATION($,'','');
…
```

```console
$ stepq strip assembly.stp --anonymize -o shareable.stp --format json
{
  "file": "assembly.stp",
  "instances": 66,
  "output": "shareable.stp",
  "strings": 115
}
$ stepq bom shareable.stp
product-1
├── product-2
├── product-3  ×2
│   ├── product-4  (2 total)
│   └── product-5  ×3  (6 total)
│       ├── product-6  (6 total)
│       └── product-7  (6 total)
└── product-8
    ├── product-9
    └── product-7  ×2

3 sub-assemblies, 5 distinct parts, 18 parts in total
```

## pmi

Lists the semantic PMI of AP242 files — the machine-readable GD&T, not
the annotation graphics — grouped under the product definition it belongs
to: datums; geometric tolerances with their characteristic, magnitude,
datum reference frame and modifiers, and the feature they apply to; and
size and location dimensions with nominal values, plus-minus bounds or
limits, and their features. Values are printed as written in the file.

```
stepq pmi FILE
```

| Option | |
|---|---|
| `FILE` | STEP file to inspect, or `-` for standard input. |

The table lists, under each product definition, its datums, then its
tolerances, then its dimensions, each in instance order. With
`--format json`, one document with `datums`, `tolerances` and
`dimensions`; with `--format csv`, one row per item, datums first, then
tolerances, then dimensions, each in instance order. A file without
semantic PMI prints `no semantic PMI`.

Library: [`stepq::pmi::pmi`](https://docs.rs/stepq/latest/stepq/pmi/fn.pmi.html).

```console
$ stepq pmi pmi-part.stp
BRACKET  #4368
  datum       A
  datum       B
  datum       C
  tolerance   position 0.75 | A | B | C  Position.1  [on COMPOSITE_SHAPE_ASPECT #235]
  tolerance   position 0.75 | A | B | C  Position.2  [on COMPOSITE_SHAPE_ASPECT #236]
  tolerance   surface profile 1.25 | A | B | C  Position surfacic profile.3  [on COMPOSITE_SHAPE_ASPECT #230]
  tolerance   surface profile 0.5 | A  Position surfacic profile.2  [on ALL_AROUND_SHAPE_ASPECT #23]
  tolerance   perpendicularity 1.5 | A  Perpendicularity.1  [on SHAPE_ASPECT #298]
  tolerance   flatness 0.2  Flatness.1  [on SHAPE_ASPECT #297]
  dimension   linear distance  [from SHAPE_ASPECT #324 to SHAPE_ASPECT #325]
  dimension   linear distance  [from SHAPE_ASPECT #328 to SHAPE_ASPECT #329]
  dimension   angle 60.0 (-0.5 .. 0.5)  [from SHAPE_ASPECT #310 to SHAPE_ASPECT #311]
  dimension   diameter 35. (-0.2 .. 0.)  [on COMPOSITE_SHAPE_ASPECT #219]
…
  dimension   diameter 25. (-0.15 .. 0.15)  [on COMPOSITE_SHAPE_ASPECT #231]

3 datums, 6 tolerances, 12 dimensions
```

```console
$ stepq pmi pmi-part.stp --format csv
category,product_definition,product,instance,type,name,value,lower,upper,datums,modifiers,target,target_type
datum,#4368,BRACKET,#37,,A,,,,,,#37,DATUM
datum,#4368,BRACKET,#38,,B,,,,,,#38,DATUM
datum,#4368,BRACKET,#39,,C,,,,,,#39,DATUM
tolerance,#4368,BRACKET,#21,position,Position.1,0.75,,,A|B|C,,#235,COMPOSITE_SHAPE_ASPECT
tolerance,#4368,BRACKET,#22,position,Position.2,0.75,,,A|B|C,,#236,COMPOSITE_SHAPE_ASPECT
…
```

```console
$ stepq pmi pmi-part.stp --format json
{
  "file": "pmi-part.stp",
  "datums": [
    {
      "instance": 37,
      "label": "A",
      "subject": {
        "instance": 37,
        "entity": "DATUM",
        "product_definition": 4368
      }
    },
…
  "tolerances": [
    {
      "instance": 21,
      "kind": "position",
      "name": "Position.1",
      "magnitude": {
        "instance": 95,
        "name": "",
        "measure": "LENGTH_MEASURE",
        "value": "0.75",
        "unit": 4361
      },
      "datums": [
        {
          "label": "A",
          "modifiers": []
        },
…
```
