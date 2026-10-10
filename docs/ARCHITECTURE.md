# Architecture

This document records the design constraints `stepq` is built on and the
evidence behind them. Read it before changing `src/model` or `src/p21`.

It assumes familiarity with the STEP entity model: the product chain,
shape representations, `next_assembly_usage_occurrence`, styled items
and the rest. Those are not explained here. The companion book
[*Inside the STEP File*](https://jchultarsky.github.io/step-book/)
covers them from first principles, with real file excerpts, and is the
foundation this document builds on. Its source lives in the
[step-book repository](https://github.com/jchultarsky/step-book).

## One-paragraph summary

A STEP file is a graph of entity instances. Everything `stepq` does is a
graph operation: pick a root, compute a closure, filter shared
aggregates, renumber, write. Writing never interprets an entity: geometry
is copied verbatim. Reading may interpret what a file states, never what it
does not (see "Reading stated geometry"). The single hardest problem is that
the closure cannot be computed by following references forward.

## The back-reference problem

Forward closure from a `product_definition` reaches exactly six
entities — `product_definition`, `product_definition_formation`,
`product`, `product_context`, `product_definition_context`,
`application_context` — and **zero geometry**. This was measured on the
STEP Tools AS1 assembly, and it holds for every AP.

The reason is that the schema is written "definition points at
definee": `product_definition_shape.definition → product_definition`,
`shape_definition_representation.definition → product_definition_shape`,
`styled_item.item → representation_item`,
`geometric_tolerance.toleranced_shape_aspect → shape_aspect`, and so on.
On a NIST AP242 part with PMI, 34% of all entities (1,491 of 4,350, in
84 types) are reachable only backwards.

Consequences:

1. `model::Graph` maintains a back index from day one. It is not an
   optimisation.
2. Closure is a fixpoint over a **rule table**: "if an entity of type T
   references anything currently retained, pull T in and forward-close
   it". The table is grown empirically from real files, and the tool
   must be able to report what it left behind (`--report-orphans`).
3. Type-level analysis of the EXPRESS schema is a starting point, not
   the answer. `styled_item` and `mapped_item` are subtypes of
   `representation_item`, so a schema-derived rule set misclassifies
   them.

## Failure is silent

Deleting one `shape_representation_relationship` from an otherwise
valid part file produces a file that Open CASCADE reads with zero
warnings, reporting the correct product name and **zero solids,
volume 0**. Forgetting `context_dependent_shape_representation` does the
same to a sub-assembly. NIST's STEP File Analyzer does not check this;
OCCT does not check this; nothing free checks this.

Therefore the test oracle is a *geometric invariant*, not a conformance
checker: read the input and every output back through OCCT and assert
`Σ volume(outputs) == volume(input subtree)` to 1e-9 relative, with
equal solid counts and equal name/colour sets. `tools/verify-occt.py`
implements this. It is a dev-time tool; OCCT is never a runtime
dependency.

Two measured caveats, both about OCCT rather than stepq
(`tools/verify-split.py` encodes them):

* A volume integrated at a different placement is not bit-identical.
  Summing placed component volumes of the NIST weldment differs from the
  assembly by 2e-7 relative, while every one of its 33 component shapes
  matches its own file exactly and placement determinants are 1 to
  2e-16. So components are compared unplaced at 1e-9, solid counts
  exactly, and only the placed volume sum at 1e-6.
* OCCT's default volume quadrature is not stable to 1e-9 on curved
  geometry (6.9e-7 on moon_buggy); use the adaptive integration with an
  error bound. And in a file with several top-level product definitions
  and no assembly structure, OCCT does not transfer every definition's
  shape (NIST FTC-09), so outputs there are bounded by the input rather
  than summed.
* Volume integration over a compound is not additive. The Creo Open Rack
  files write a surface model (`shell_based_surface_model` with open
  shells) next to each solid: one housing's solid integrates to 22,933.83,
  its open shells to 0, and the two together to 23,063.60. Placed in its
  assembly with 22 other solids, whose every one matches its own file to
  1e-15, the assembly integrates to 27,122.62 against components summing to
  27,096.39, a 1e-3 difference that is all Open CASCADE's. Volumes are
  therefore summed solid by solid.

## Entities stay untyped

AP242 ed4 declares 2,407 entity types, 248 of them with multiple
supertypes. Seventeen real AP242 files use 228 types in total. Generating
a struct per type buys compile time, API churn on every schema edition,
and nothing the graph operations need.

Instances are stored as `(type name(s), attribute token list, original
byte range)`. The EXPRESS schema is loaded as data, at run time, and used
only for validation: `stepq lint --schema` checks entity types, attribute
counts and aggregate lower bounds (an empty `SET[1:?]`), and the test
suite checks every attribute position in the closure rule table against
it. Nothing else reads the schema; `split` needs none.

## Verbatim output

Unchanged entities are written from their original source text. Numbers
are never re-parsed and re-printed; strings are never re-escaped. Only
`#id`s change, and the renumberer must tokenise strings first —
`PROPERTY_DEFINITION(...,'volume of #602',...)` is a real line in a real
sample file.

## Assembly structure

Two parallel trees: products (`product` → `product_definition_formation`
→ `product_definition`, linked by `next_assembly_usage_occurrence`) and
shapes (`shape_representation` linked by
`context_dependent_shape_representation` →
`(shape_representation_relationship, representation_relationship_with_transformation)`
→ `item_defined_transformation` → two `axis2_placement_3d`). A second
placement mechanism, `mapped_item` / `representation_map`, also occurs.

* `transform_item_k` is an item of `rep_k` (AP242 WR2 of
  `representation_relationship_with_transformation`), and the NAUO, not the
  relationship's order, decides which of `rep_1`/`rep_2` is the component's.
  The transform carries the component's frame onto the assembly's:
  `M(assembly-side item) · M(component-side item)⁻¹` — `M(item_2)·M(item_1)⁻¹`
  in the usual order, `M(item_1)·M(item_2)⁻¹` in the 131 reversed
  Pro/ENGINEER usages of the fixtures. The component-side item is the
  identity in all 3,458 fixture usages. Where neither item is the identity,
  Open CASCADE 8.0.1 composes the other way round, so stepq refuses such a
  usage (`PlacementError::Divergent`).
* Real files reverse `rep_1`/`rep_2`. OCCT carries a
  `CheckSRRReversesNAUO` workaround dated 1998. **The NAUO wins.**
* In AP242, NAUO `related`/`relating` became a SELECT. Part 21 does not
  tag entity-valued SELECTs, so the record is byte-identical to AP214;
  dispatch on the target's type, not the declaring attribute.

## Extracting a sub-assembly

The transform that places sub-assembly B inside A lives on the NAUO
that is a property of the *usage*, not of B. When B becomes a root, that
NAUO is simply not retained and the transform disappears with it. No
matrix arithmetic. B comes out in whatever frame its author modelled it
in — which is not necessarily the origin. Do not promise otherwise.

## Things that must be filtered, not copied or dropped

Several entities aggregate items from many parts in one list:
`presentation_layer_assignment`, `product_related_product_category`,
`mechanical_design_geometric_presentation_representation`,
`draughting_model`, every `applied_*_assignment`, `invisibility`. Copied
wholesale they dangle; dropped they lose colours or categories. The
list is rewritten to the retained subset, and the entity is dropped only
if the subset is empty.

A filtered list is not followed, with one exception. A `draughting_model`
lists the presentation of many PMI elements, but its saved views
(`camera_model_d3` and relatives), the shapes shown in them (`mapped_item`)
and its notes exist only as its items; nothing else refers to them. So the
rule for `draughting_model` follows items of those types, except an
`annotation_plane` whose elements semantic PMI is associated with
(`draughting_model_item_association` and relatives): that plane belongs to
the PMI's product and comes with it. Following every plane copied the
callouts of three top-level definitions into each other's files in NIST
FTC-09.

## What the rule table covers

`split --report-orphans` reports what no output holds. The table was grown
from those reports on every fixture, the CAx-IF recommended practices (PMI
Representation and Presentation, PMI Polyline Presentation, External
References, Tessellated Geometry) and the schemas, and no rule was added
without checking its position in AP242 ed. 4, AP214 and AP203 ed. 2.

On the 48 fixtures under 16 MB, instances in no output went from 43,534 to
1,559; on all 55 fixtures, including the Open Compute assemblies up to
207 MB, 2,969 remain, and no entity type grew along the way. What the
growth brought in, by family: PMI presentation (annotation planes, draughting
relationships, per-annotation validation properties, callout
relationships, display formats), saved views (AP242 cameras, Creo
presentation sets, areas and views), supplemental geometry, notes and
polyline or tessellated annotation reached through draughting models,
tolerance zones, composite tolerances, referenced standards documents,
addresses, and two subtypes the table had missed
(`directed_dimensional_location`, `default_model_geometric_view`, whose
`of_shape` is its seventh attribute). The 45 core fixtures still split to
Open CASCADE's solid counts and volumes.

None of what remains is left behind: all 2,969 instances are data nothing
refers to, directly or through other such data (`model::unreachable`) —
1,827 colours no style uses, 639 unused `dimensional_exponents`, 69
pre-defined colours, 40 product categories, dates, and the saved views of
the NIST FTC-09 draughting model no product reaches. No product can own
those, so the report lists them apart from what is left behind. Not handled on purpose: a plain
`representation_relationship` between AP203 draughting models (a rule on it
would also match other relationships), and the Creo saved views' mapped
items as back references, which would pull a parent's placements into a
child placed through `mapped_item` (the NIST moon buggy).

## Things that must not be merged

`representation_relationship_with_transformation` has a WHERE rule:
`rep_1.context_of_items :<>: rep_2.context_of_items`. Parent and child
must not share a `geometric_representation_context` even when the two
are byte-identical. OCCT reads a merged file without complaint; a
conformance checker fails it. Never deduplicate contexts.

## Multi-body split

A part whose shape holds several `manifold_solid_brep` items (Creo writes
7 and 22 into one representation in the Open Rack fixtures) is split by
`split --bodies` into one file per solid without synthesising anything:
the part is extracted with its other solids excluded
(`model::extract_excluding`). Their faces, edges and styles are never
reached, and every extracted instance that listed an excluded solid —
both shape representations that Creo writes, a layer — is pruned, so its
list keeps only the remaining solid. A reference to an excluded solid
that is not a list item cannot be pruned; the write then fails instead of
producing a dangling file. Each body file keeps the part's product, so
it reads as the same part with one body.

Not handled, and refused rather than guessed: disconnected lumps inside a
single (schema-illegal) shell, which would need union-find over shared
`edge_curve` references, and a `brep_with_voids` whose outer shell is
disconnected, which needs point-in-solid to assign its voids.

## Master files

`split --master` writes each assembly as a master file that refers to its
components' files, following the CAx-IF Recommended Practices for
External (Element) References 3.1, §6.1. Attribute order comes from the
schema, not the document: two of its own examples are wrong
(`applied_document_reference` has three attributes, and one example
reverses them).

A master keeps its own instances and, of each component, a stub: the
product, formation and definition, the shape definition, and its shape
representations reduced to their placements. It is `extract_excluding`
from the assembly with every component definition, every non-placement
item of their shape representations, and every simple
`shape_representation_relationship` to those representations excluded.
That last exclusion is not optional: exporters such as Unigraphics tie a
placement-only representation to the one holding the solid with such a
relationship, and without it the rule table pulls the geometry back in.
Per component the file gains, numbered after its highest instance name:
`document_type`, `document_file` (id = file name),
`document_representation_type('digital')`, `identification_role('external
document id and location')`, `external_source` (empty: same folder),
`applied_external_identification_assignment` (the file name, which Open
CASCADE reads first), `applied_document_reference` to the component
definition, `object_role('mandatory')` with its `role_association`, and the
'external definition' property linking the document to the stub shape.

Open CASCADE reads the top master of the AS1 assembly from three exporters
(Unigraphics, CADDS, Pro/ENGINEER), nested masters included, with exactly
the original solid count and volume, and every master file holds no
solid. It does not attach external files to components placed through
`mapped_item`s (the NIST moon buggy): its reader ties an external file to
the component definition, which mapped items bypass.

## Assembling

`assemble` is the inverse of a master. Each `applied_document_reference`
to a `document_file` names a file (the external identification's
`assigned_id`, else the document id, as Open CASCADE reads it) and the stub
definition it stands for. The stub's product, formation, definition, shape
definition, its link to a shape and its shape representations are paired
with the same instances of the file's definition with that product id;
references to them are rewritten with `p21::Replacements`, and the stub
and the reference entities are dropped. Component files are written after
the master with `Numbering::Offset`, past every name already used, so
nothing but the redirected tokens changes.

A file is merged once, however many masters name it: the AS1 nut is used
by the rod assembly and by the nut-and-bolt assembly, and assembling
nested masters one file at a time brought it back twice. Merged files are
therefore remembered by name, and a file that refers back to one being
merged is an error rather than a loop.

Measured on the AS1 assembly from four exporters (Unigraphics, CADDS,
Pro/ENGINEER, and NIST's `as1_pe`): splitting into masters and assembling
the top master gives the original products and component quantities
(`stepq diff`), and Open CASCADE reads the same solid count, volume,
names and colours as from the original file.

## Validation properties

Geometric validation properties (volume, area, centroid) cannot be
recomputed without a kernel, so an extraction must never carry one that
describes something else. It does not: the rule table takes only the
properties of each extracted definition and its shape aspects, and an
assembly's own properties describe its own sub-tree — exactly what its
extraction holds. Nothing needs dropping on re-root.

Measured on the AS1 splits from two exporters (CADDS via Theorem, and
Unigraphics): in all 18 output files, the root definition's declared
volume matches the volume Open CASCADE computes for that file to the
exporter's precision (for example 108452.2 declared, 108453 computed for
the L-bracket sub-assembly). `stepq props --kind validation` reads the
declared values.

## Reading stated geometry

> stepq reads what a file states and never computes what it does not. A
> point, a direction, a radius, an axis, a placement and a unit factor are
> stated. Reading them may normalise a direction, apply the stated unit
> factor of the value's own context, and compose stated placements.
> Intersections, projections, trimmed areas, arc lengths, volumes, bounding
> boxes, tessellation and healing are computed, and they stay out: use a
> kernel. A value stepq cannot read with certainty is reported, as an
> "other" variant or an error with a reason. It is never guessed, never
> defaulted beyond the schema's own defaults, and never skipped. Reading
> never changes output: unchanged entities are still written byte for byte.

Reading is as silent as writing when it goes wrong: a length read in the
wrong unit is a number like any other. So every reader is checked against
Open CASCADE's reading of the same files (`tools/verify-read.py`, one check
per reader, on the core fixtures in CI and on files only Open CASCADE writes,
generated there by `tools/gen-occt-cases.py`), and every reader shares one
way of reading records (`src/record.rs`): which partial of a complex
instance holds an attribute, the exact attribute count of every record it
reads, INTEGER tokens accepted where REALs are expected, non-finite numbers
refused, a complex instance that repeats a partial entity (and so states its
attributes twice) refused rather than read from either, and one table of
which face, shell, solid or model holds what.

### Units

A length is read in millimetres, a plane angle in radians and a solid angle
in steradians, converted with the factor of the unit that the value's own
representation context states (`stepq::units`). A context's three kinds are
read, or refused with a reason, each on its own, so that a broken angle unit
blocks only what needs an angle:

* An SI unit's factor comes from an exact table of the 16 prefixes: a
  millimetre is exactly 1, a centimetre exactly 10.
* A conversion-based unit's factor is its stated value times its base
  unit's factor, through chains up to 8 deep. Nothing is snapped to 25.4 or
  π/180: the fixtures state the degree with several factors, down to
  `0.0174532925`, and each is used as written, as Open CASCADE does. The
  unit's name is reported, never interpreted.
* Every statement of a unit's kind must agree — its kind record, its SI
  name, its base unit, its factor's measure type and value type, and its
  dimensional exponents — or the unit is refused. Every unit a fixture
  context lists agrees.
* A context that states no unit of a kind has none (`Missing`); several of
  one kind must have bit-equal factors (`Ambiguous`); a listed unit that
  cannot be read refuses its kind, or every kind if its kind cannot be told,
  even when a readable unit of that kind is listed too (`Unreadable`). Only
  a valid unit of another quantity (a kilogram, a second) is ignored;
  anything else listed whose kind cannot be told — a measure with unit, an
  SI name outside the schema's `si_unit_name`, a mass unit over a metre —
  refuses every kind. A unit or uncertainty set that is `$`, empty or lists
  anything but references is malformed, never empty.

Uncertainties are per context, not per file, and each is converted with its
own unit, which its measure type and value type must agree with:
Pro/ENGINEER's AS1 states a 'closure' of 0.115, 0.363 or 0.672 mm
in its seven contexts. Exporters name them differently
(`distance_accuracy_value`, `closure`, `MODEL_ACCURACY`, …), and stepq
reports names and values without choosing between them.

A solid, shell or face takes the context of every representation that lists
it, directly or through the faces, shells, solids and surface models that
contain it (an `oriented_face` and a `connected_face_sub_set` among them).
Every such representation must be readable and their contexts must agree
kind by kind and in dimension. What the walk cannot follow is refused, not
stepped over: a shell or solid written as a complex instance, or an instance
that lists the item next to a reference that is not a context, could hold it
in another context, so the item is `NotFollowed` even when its other holders
agree. Each instance on the way is read once, so the walk stays linear in
the file however many shells, representations or contexts share an item.

A placement frame is scaled in the context its position assigns (ISO
10303-43): `transform_item_k` in `rep_k`'s context, a `mapping_origin` in its
`mapped_representation`'s, a `mapping_target` in the context of the
representation that lists the mapped item. Not in the context of whichever
representation lists the frame: real files list a frame in neither or both
(in the fixtures other than Project Olympus, `transform_item_1` is listed in
both representations in 39 of 1,915 placements, and only 7 of 188 mapped
items list their target in the representation that holds them).
Transforms are composed in millimetres, and a zero length needs no unit, so
an origin frame reads even where no length unit can be read. This matters:
the Open Rack and Project Olympus assemblies place inch or foot components
in millimetre assemblies 42 times.

On all 55 fixtures, the 1,670 contexts that assign units read 1,551 lengths
in millimetres, 99 in inches and 20 in feet, 36 plane angles in radians and
1,634 in degrees; 13 state no solid angle unit and 9 no uncertainty. Every
one of 24,362 representations, 1,348 solids and surface models and 342,733
faces resolves its units. Open CASCADE 8.0.1 reads every factor and
uncertainty bit for bit the same, but one.

### Geometry values

`stepq::geometry` reads the points, directions, axis placements, elementary
surfaces, lines, circles and surface curves a file states (ISO 10303-42),
through a `Reader` tied to a representation, or to a solid, shell or face
whose holding representations must agree. There is no reader built from bare
factors and no unscaled path: a cone's semi-angle of 59 is 59 degrees in a
degree context, 1.0297 is 59 degrees in a radian one, and the 442 conical
surfaces that the 55 fixtures state as 0.57, 1 or 2 degrees stay degrees (28
in the core set, the weldment's 8 and NIST FTC-07 (rd)'s 20; the rest are
Creo's). A length is converted when it is read, and a zero length needs no
unit; a plane angle is needed only by a cone. A direction is normalised,
dividing by its largest ratio before squaring so that ratios of 1e-200 or
1e200 stay in range, and never scaled: NIST FTC-08 (rc) writes 1,159
directions of length 1/25.4, as if a converter had converted them as inches.

Axes are the schema's `build_axes`, with `first_proj_axis` as AP203 edition
2, AP214 edition 3 and AP242 edition 4 define it: z is the stated axis or
(0, 0, 1); x is the stated `ref_direction` projected off z, or by default
(1, 0, 0) projected off z, or (0, 1, 0) when z is exactly ±(1, 0, 0); y is
z × x. A defaulted x is reported as such (`ref_direction_stated`), and is
computed so that an axis just off ±X still gives a square frame: the
projection's component along the default, 1 − z_k², is written as the sum of
the other two squares. AP203 edition 1 (`CONFIG_CONTROL_DESIGN`) tests only
z ≠ (1, 0, 0), so for z = (−1, 0, 0) without a `ref_direction` its own
schema leaves x undefined. stepq reads the later editions' default and
reports `ref_direction_stated = false`: five plane positions in gear,
clevis21 and valve, where x sets only the surface's parameterisation.
Placements refuse a defaulted x where it matters.

Read as stated: the five elementary surfaces, `line`, `circle` in a 3D
placement, and `surface_curve`, `seam_curve` and `intersection_curve`, whose
`curve_3d` is read one level deep and whose `master_representation` is
reported. ST-ACIS writes intersection curves whose 3D curve is the master,
Matra's AS1 surface curves whose second pcurve is, and Open CASCADE every
edge as a surface or seam curve whose first pcurve is; under a pcurve master
the schema prefers the pcurve, which stepq does not read, so trusting
`curve_3d` there is the caller's decision. Every other entity, and every
complex instance (in the fixtures only rational B-splines are complex), is
`Other` with its type. Of the 342,733 face surfaces and 979,505 edge curves
that the representations of the 55 fixtures reach, all read; 10,915 surfaces
are `Other` (B-spline surfaces, 1,581 of them complex, and surfaces of
linear extrusion and of revolution), and 73,581 curves are or wrap one
(B-splines, 128 of them complex, and 224 ellipses). Every record read must
have its schema's attribute count, an INTEGER token is a number, and a typed
measure such as `LENGTH_MEASURE(5.)` where the schema has a plain number is
refused. A
radius must be positive both as written and in millimetres, where a tiny
unit can take it to zero (a cone's may be zero); every length must stay
finite once converted; a vector's magnitude must not be negative, a
direction not all zero, and the policies below bound a stated
`ref_direction` and a cone's semi-angle. Errors name the innermost
instance: a zero direction under a cylinder's placement is reported at the
direction, and a reference to an undefined `#id` at the instance holding it.

`tools/verify-read.py surfaces` and `curves` hold this to Open CASCADE.
Open CASCADE does not bind faces or edges to the entities it reads them
from, so each file's surfaces and curves are compared as multisets, one to
one, by keys that x does not enter (a plane's signed normal and offset, a
cylinder's axis line and radius, a cone's apex, signed axis and semi-angle,
a circle's centre, signed normal and radius, a line's direction and where it
runs). On CI's set, the 45 core fixtures and the seven generated files,
every one of 9,961 faces and 23,910 edges reads, the face kinds agree in
count, and all 9,751 elementary surfaces and 21,701 lines and circles match
Open CASCADE's to 4.2e-16. On all 55 fixtures with the generated files, the
342,788 faces and 979,612 edges of shape representations read, and all
331,873 elementary surfaces and 906,031 lines and circles match to 5.6e-16.

### Placements and occurrences

`model::Placements` reads the transform each assembly usage states, from the
component's coordinates to the assembly's, in millimetres, and composes them
into occurrences: every place a definition occurs under a root, depth first
in file order, each with its path of usages, the representation its
transform maps from and its transform to the root. A sub-assembly used twice
occurs twice; separate roots are unrelated frames. The iterator is lazy and
holds only the current path, so a crafted file that reaches a sub-assembly by
exponentially many paths costs only what a caller takes from it.

Each frame is read through `geometry::Reader` in the context its position
assigns (above). An `item_defined_transformation` gives `M(assembly-side
item) · M(component-side item)⁻¹`, the usage deciding the sides ("Assembly
structure"). A `mapped_item` gives `M(mapping_target) · M(mapping_origin)⁻¹`;
Open CASCADE agrees there. Mapped items name a component but not a usage, so
`ProductStructure` pairs them with that component's usages in file order:
the set of transforms is exact, and which usage gets which follows the file.
It reads a mapped item written as a complex instance, whose `MAPPED_ITEM`
partial holds the source and target at its first two positions (before
0.5.0 it read them one position on and left such usages unplaced), and the
items of a complex representation; no fixture has either.

What cannot be read as stated is a `PlacementError` for that usage and for
every occurrence below it, and for nothing else:

* `Unplaced` (no `context_dependent_shape_representation`, and not as many
  mapped items as usages), `Quantified` (a quantity other than 1, or one that
  cannot be read: one placement cannot place several occurrences) and
  `NoTransformation` (a plain `shape_representation_relationship`).
* `Unsupported`: every `cartesian_transformation_operator`, which can scale
  and mirror, a `functionally_defined_transformation`, complex
  transformations, relationships and maps, a complex mapped item with more
  than a representation item's partials (Creo's scaled camera images), and a
  frame that is not an `axis2_placement_3d`. None occurs in the fixtures.
* `Inconsistent`, read strictly: `rep_1` and `rep_2` must be exactly one of
  the component's and one of the assembly's shape representations,
  `transform_item_k` must be listed directly in `rep_k` (WR2 asks only that
  `rep_k` use it, perhaps through another item; no fixture needs that), a
  usage must have one `context_dependent_shape_representation`
  (`ProductStructure` identifies the first), one of the assembly's shape
  representations must list a mapped item, and a path must place each
  definition through the representation that holds its components, a root's
  usages agreeing on it. In the fixtures `transform_item_1` is listed in both
  representations 39 times (the AS1 files by AutoCAD, Matra and Unigraphics
  share one identity frame, as Open CASCADE's writer does), and never only
  in `rep_2`.
* `Divergent`, where ISO 10303 and Open CASCADE place a component
  differently (below); `Frame`, a frame that cannot be read, a non-zero
  location whose unit cannot be read among them, or a transform no longer
  finite once composed; `Cycle`; and `TooDeep`, past 1,024 levels.

On all 55 fixtures the 3,481 usages read: 3,327 through relationships in the
usual order, 131 reversed (as1_pe 9, vaccase 14 and weldment 108, all
Pro/ENGINEER files in inches) and 23 through mapped items (the moon buggy).
So do all 4,359 occurrences, up to 7 levels deep (Project Olympus 1U), and
nothing is refused. 42 of the usages place an inch (38) or foot (4)
component in a millimetre assembly.

`tools/verify-read.py occurrences` holds this to Open CASCADE's XCAF reading
of the same files: free shapes, their component labels, and locations
composed from the root down. Open CASCADE binds no label to the entity it
reads it from, so occurrences are joined by the names on their paths — the
name Open CASCADE gives each component label from its NAUO (description,
else name, else id) and the product's — one to one, which tells apart the
AS1 assembly's three nut-and-bolt instances NBA1, NBA2 and NBA3; by value
among occurrences sharing a name; and by value at the same depth for the
moon buggy's 29, which Open CASCADE leaves unnamed. Open CASCADE also lists
the solids, shells and compounds of a part's shape as labels of their own;
unmatched, they are dropped. A rotation entry must agree to 1e-12 and a
translation to 1e-9 of max(1 mm, |t|), and an occurrence left unmatched on
either side, or refused, fails. On CI's set, the 45 core fixtures and the ten
generated files (`tools/gen-occt-cases.py`: the parts, a nested assembly
Open CASCADE writes, whose 8 occurrences match, and two divergence cases made
from it), the 375 occurrences not refused match to 1.1e-16 and 3.2e-14 mm,
and the divergence cases refuse their 4. Those counts are pinned
(`--expect`): an occurrence stepq dropped from the moon buggy would leave
only an unnamed label behind, and every file would still agree. On all 55
fixtures, every one of the 4,359 occurrences matches Open CASCADE's, to
2.2e-16 and 1.3e-13 mm.

### What is not read

Three forms, never a silent default. An entity stepq does not interpret is
an `Other` value naming its type, for the caller to treat as unknown. A value
asked for that cannot be read as stated is a typed error, one type per
module (`units::UnitError`, `geometry::GeometryError`, …), naming the
innermost `#id` and why:
`Unsupported` for valid STEP stepq does not read (a mass unit, a
context-dependent unit), `Malformed` for what contradicts the schema.
`stepq::Error` wraps each transparently, but the per-module errors are what
results stored per item hold, because they are `Clone` and `PartialEq`.
Walks that return many things list what they did not follow.

### Known differences from Open CASCADE

* Open CASCADE reads one uncertainty per context, the smallest length one,
  and takes its stated value as if it were in the context's length unit;
  stepq reads every stated uncertainty, each in its own unit. They agree on
  every fixture, where each uncertainty is stated in its context's length
  unit.
* An inch defined as 2.54 centimetres is 2.54 × 10 = 25.4 mm in stepq and
  25.400000000000002 in Open CASCADE (NIST FTC-07), one unit in the last
  place; the oracle compares at 1e-12.
* The default x axis. Where `ref_direction` is omitted, Open CASCADE 8.0.1
  builds x with `gp_Ax2`'s own rule, not the schema's `first_proj_axis`, and
  the two differ for most axes other than (0, 0, 1). Measured through Open
  CASCADE's STEP reader: (0, 0, −1) gives x = (−1, 0, 0) where the schema
  gives (1, 0, 0); ±(1, 0, 0) gives ±(0, 0, 1) where the schema gives
  (0, 1, 0); ±(0, 1, 0) gives ±(0, 0, 1) where the schema gives (1, 0, 0);
  and (1, 1, 1)/√3 gives (0.707, 0, −0.707) where the schema gives
  (0.816, −0.408, −0.408). They agree for (0, 0, 1) and, for example,
  (0.6, 0, 0.8). stepq follows the schema and says x was not stated. In the
  fixtures only plane and circle positions, and frames that omit the axis
  too, omit it; 903 of the 1,328 that state an axis get a different x in
  Open CASCADE, all 360 on (0, 0, −1) among them. In a placement x turns
  the component about its axis, so `Placements` refuses a frame that omits
  `ref_direction` unless its axis is exactly +Z (`Divergent`,
  `DefaultedRefDirection`); no fixture placement omits it. On a generated
  case, a sub-assembly on (0, 0, −1) with no `ref_direction`, Open CASCADE
  places it with x = (−1, 0, 0).
* An `item_defined_transformation` neither of whose items is the identity.
  ISO 10303 carries the component's frame onto the assembly's,
  `M(assembly side) · M(component side)⁻¹`; Open CASCADE 8.0.1 composes
  `M(item_1)⁻¹ · M(item_2)`. They agree when either item is the identity, as
  the component-side item is in every fixture usage. On a generated case, a
  plate whose component-side item is a frame at (10, 0, 0) on axis x,
  placed by a translation of (0, 0, −10), Open CASCADE puts the plate's
  origin at (0, −10, −10) and ISO at (0, 0, −20). stepq refuses such a usage
  (`Divergent`, `BothFramesPlaced`). Mapped items do not differ: Open
  CASCADE takes the mapping origin off first, as ISO does.
* Cone semi-angles. Open CASCADE clamps a semi-angle at or below 1e-12 to
  1e-12, a negative one included, and drops a cone at or above π/2; stepq
  refuses any outside [1e-6, π/2 − 1e-6] rad. No fixture has one, so the
  oracle cannot see them and unit tests cover them.
* Healing. Open CASCADE adds a seam to a periodic face that the file bounds
  only by closed loops (a cylinder between two circles) and splits the
  closed edges there; it reads a wireframe's curves as edges too. Neither
  is stated, so `curves` accounts for them instead of matching them: what
  Open CASCADE holds beyond stepq is excused if it is a seam of its face
  (one the file states included) or equal to an edge stepq reads. An edge
  stepq failed to read that is a seam, or equal to another edge, is
  therefore caught only by the tallies, which CI pins (`--healing`): on its
  set, 5 lines and circles that are seams, 267 pieces of split circles, and
  770 B-spline seams and pieces.

### Policies

Limits stepq applies beyond what the schemas state are kept in
`src/policy.rs`, each with its evidence, and none refuses anything in the 55
fixtures:

| Policy | Value | Used by | Evidence |
| --- | --- | --- | --- |
| Axis nearly parallel to a stated `ref_direction` | refused when the sine is below 1e-6 (the schema refuses only exactly 0) | geometry, placements | smallest sine in the fixtures: 0.99999999999 |
| Cone semi-angle | refused outside [1e-6, π/2 − 1e-6] rad once converted | geometry | 90 × 0.0174532925 is π/2 − 1.79e-9, which would otherwise read as a valid cone |
| Conversion-unit chain | 8 deep | units | deepest in the fixtures: inch → centimetre → SI |
| Assembly depth | 1024 | product structure, placements | deepest fixture assembly: 7 levels |

## Non-goals, and why

Tessellation, intersections, trimmed areas, mass properties, bounding
boxes and healing all require evaluating curves and surfaces. Open
CASCADE's STEP translator is ~300k lines on top of ~90k lines of healing
and ~80k of intersection/projection. That is not a module of this project;
it is a different project. Applying a unit factor the file states is
reading, not conversion, and is in scope from 0.5.0.
