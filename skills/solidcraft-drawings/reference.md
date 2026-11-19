# SolidCraft drawings reference for agents

Command shapes for drawings, where things land on the sheet, the traps, and one worked example. The
example runs in CI (`drawings_skill_example_builds_as_documented` in `crates/mcp/src/tests.rs`), so
its numbers stay honest.

Paper positions are millimetres from the sheet's lower-left corner; model points are `[x, y, z]` in
the design's frame. `describe_command` gives any command's full docs; `list_commands {"prefix":"drawing."}`
lists them all.

## What a professional sheet is
Read this before placing anything. The commands below only make marks; these rules make a drawing.

**The test.** Someone who has never seen the model makes the part from the sheet alone, and an
inspector checks it from the same sheet. Every feature has its size and its position, once.

**Views**
- One part per sheet. The fewest views that show every feature: a turned part needs its side view
  and at most an end view; a plate its face and an edge; a prismatic part three.
- The main view shows the part as it is made or as it sits: a turned part's axis horizontal, the
  largest diameter towards the left chuck end or as the user's other drawings have it.
- What the main view hides (a bore, an internal thread, a pocket's depth) is shown by a section,
  not by hidden lines with dimensions on them. Hidden lines (`visual_style: "visible_hidden"`) are
  for understanding only.
- A small isometric (half the main scale or less) in a free corner helps the reader. It carries no
  dimensions and never touches another view.
- Pick the scale so the main view takes about half the sheet's width, from 1:1, 2:1, 5:1, 10:1 or
  1:2, 1:5, 1:10. Every orthographic view of the part has the same scale. Leave 35 mm or more
  between views and 25 mm to the border for dimensions.
- A projected view left where `drawing.view.projected` puts it stays lined up and unlabelled. Placed
  by hand (`place: {at}`) it gets a "VIEW B / SCALE" label that then needs room: prefer the default,
  and a smaller scale or a larger sheet when it doesn't fit.

**Dimensions**
- A size is a dimension on the view, read from the model, never typed text. A sheet whose sizes are
  a list of notes is not a drawing: nothing ties a line of text to a feature, and it doesn't follow
  the model.
- Each size once, on the view that shows the feature true and in profile. No size that can be
  added up from others in the same chain (leave one out, or give the overall as the check).
- Diameters: the `cylinder` kind on the side view (reads Ø between the silhouettes), or `radial`
  with `diameter: true` on a circle seen face on. A partial circle reads R unless `diameter` is set.
- Lengths: from one datum face (the face the part locates on) or as one row end to end, then the
  overall length in a row outside it. Don't mix the two schemes on one view.
- Across flats, slot widths and steps: the `faces` kind between the two faces.
- Dimension lines sit outside the view's outline: the first row 10 mm from it, each further row
  10 mm more, shorter dimensions nearer the part. A diameter's line goes beside the feature it
  sizes, not across the view to the far end. No dimension line crosses another; no text sits on a
  line or on the part.
- `decimals` says the tolerance: whole millimetres 0, tenths 1, a fit's size 2 or 3. `8.00` on a
  free length asks for a hundredth nobody needs.
- A tolerance or fit the user gave goes on its dimension with `override_text` (`"Ø10 h9"`), which
  Check Drawing reports as `overridden`: expected. Never tolerance a dimension on your own judgment.

**Callouts and notes**
- A thread, a chamfer, a knurl, a surface finish: one note with a leader ending on the feature
  (`anchor: {edge: {view, edge}}`), placed outside the view: `M10 × 1.5`, `C0.5`, `Ra 0.4`.
- What applies to the whole part goes in one numbered note, `NOTES` then `1. …`, near the title
  block or the lower left: deburring, finish, heat treatment, the standard its general tolerances
  follow. Six lines at most.
- Nothing else is written on the sheet: no heading with the part's name, no "provisional", no
  quantity or scale line, no caption under a view, no restating a dimension in words. The title
  block has the title, number, material, scale and revision.
- Text is 3.5 mm (the default); only a view label is larger.

**Title block**
- Link the sheet to its part (`documents`) so title, material and mass are read, not typed. Type
  over a cell (`title_block_overrides`) only for what the user gave: a drawing number, a title that
  differs from the body's name, a finish.
- A long name runs out of its cell: `drawing.inspect`'s `title_block_cells[].notes` says which.
- Ask for what is empty (owner, drawn by, date) rather than leaving it or making it up.

**Ask, don't invent.** Tolerances, fits, surface finishes, materials, part numbers and revision
letters come from the user or from the project's own files (`project_search`, `project_read`). If
the sheet needs one you don't have, ask once, listing them all.

## A turned part, step by step
The order that works for a shaft, pin, bush or spacer (the male shaft of the example below took
these steps):
1. `body_topology {"body": name, "faces": "cylinders"}`: each cylinder's radius and its `extent`
   along the axis give every diameter and every shoulder. `model.faces` gives a `point` on each flat
   face (the shoulders, the ends, any flats); `model.edges` gives each edge's `mid` for leaders.
2. A new sheet with its `size`, linked to the part. Main view: a base view with a `view` of
   `{direction, up}` chosen so the axis runs along the paper's X (`direction` square to the axis,
   `up` square to both), `source: {occurrences: [[id]]}` for a part inside an assembly.
3. End view: `drawing.view.projected` to the `right`, only when the part has flats, holes or a
   shape the side view can't size.
4. One row of lengths between neighbouring shoulders with the `faces` kind, `direction: "horizontal"`,
   then the overall. Move the row to `min.y - 10` and the overall to `min.y - 20` of the view's
   `boundary_mm`.
5. Each diameter with `cylinder`, its line moved just past the end of the view nearest that
   feature (`min.x - 10` or `max.x + 10`), further out for the next.
6. On the end view: across flats with `faces`, a collar with flats as `radial` with
   `diameter: true`, its text moved off the view (`text_to_mm`).
7. Threads as leader notes from the thread's end edge. A modelled thread draws every turn; say so to
   the user if it clutters: a cosmetic thread draws the conventional two lines instead.
8. An internal feature: `drawing.view.section` with a `full` line along the axis, and its depths
   dimensioned on the section.
9. Check, list overlaps from `drawing.scene`, show the sheet, screenshot, fix, repeat.

## Looking at your own sheet
- `drawing.sheet.show {"sheet": name}` puts a sheet on the window's sheet page; the page otherwise
  keeps showing whichever sheet it had, and the user sees none of your work. Then
  `screenshot {"source":"window"}`.
- `drawing.scene {"sheet": name}` → `objects: [{object, kind, bounds_mm}]`. Two boxes overlap when
  `a.min.x < b.max.x && b.min.x < a.max.x` and the same in y. A dimension's box includes its
  extension lines, so dimensions of one row always touch: test notes, view labels and tables against
  everything, and each dimension's `layout.text_boxes` (`drawing.dimension.inspect`) against the
  other texts and the views' `boundary_mm`.
- Before you say it's done: every feature sized and placed? Any text on a line or on the part? Any
  typed size? Anything on the sheet the title block already says? Any empty title block cell you
  could have asked about?

## Ids
Drawing, sheet, view, dimension and note ids are random (`"259g7shs2tfj1"`), not counters, so a
batch can't name what an earlier step of the same batch made. Build in three batches: the sheet and
views; then the title block and dimensions; then moves. A sheet can also be named (`"Sheet 1"`).

`drawing.create` and `drawing.open` reply with the whole drawing (about 6 kB, most of it the title
block's definition). `drawing.inspect` gives the same; call it only when you need the lists of views,
dimensions and notes on each sheet.

## Commands

### The drawing
- `drawing.create`: `name?`, `template?` (`drawing.template.list`: ISO A4 to A0, for a part or an
  assembly; default A3 for a part), `projection?` (`first` | `third`, default third),
  `standard_views?: true` (places front, top, right and iso itself), `units?`, `language?`, `font?`.
  Opens the drawing. Reply: the drawing, with `sources: [{id}]` (the design itself is the first) and
  `sheets: [{id, name, size_mm, …}]`.
- `drawing.list`, `drawing.open {"drawing": id}`, `drawing.close`. While a drawing is open the
  model's commands wait; close it before changing the part.
- `drawing.set`: `name`, `projection`, `units`, `font`, `line_group`, `updates` (`automatic` |
  `deferred`).
- `drawing.sheet.add`: `name?`, `size?` (`{"standard":"a2"}` or `{"custom":{"sides_mm":[w,h]}}`),
  `orientation?`. `drawing.sheet.set` changes one, and links it to the part it documents.
  `drawing.sheet.show {"sheet": name}` shows one in the window.

### The title block
The built-in block reads fields; nothing needs typing when its sources are set:
- **Title, Material, Mass:** the component the sheet documents. Set
  `drawing.sheet.set {"sheet":"Sheet 1","documents":{"source": <sources[0].id>, "component": 0}}`
  (component 0 is the design). Title is the design's name; Material needs `material.assign` on the
  bodies; Mass follows from the material.
- **Identification number, Legal owner:** `drawing.properties.set {"values":{"part_number":…,"organization":…}}`.
- **Created by, Approved by, Date of issue:** the newest revision row:
  `drawing.revision.add {"set":{"description":…,"designer":…,"checker":…,"date":"YYYY-MM-DD"}}`.
- **Finish:** a free-text property the design itself doesn't have; type it with
  `drawing.sheet.set`'s `title_block_overrides: {"finish": "Anodised"}` when the user gives one.
  Check reports typed-over cells as `overridden`; that is expected.

### Views
- `drawing.view.standard`: `sheet`, `view?` (the main view, default `front`), `scale?` (`"1:2"`;
  default the largest ISO scale that fits), `gap_mm?` (default 20), `iso?` (default true). Reply
  `{view, top, right, iso, sheet, scale, outside?}`: `view` is the front view's id. In third angle
  the top view sits above the front and the right view to its right. A 20 mm gap holds one row of
  dimensions; 35 holds two.
- `drawing.view.base`: `sheet`, `view?` (`front`, `top`, `iso`, …), `scale?`, `position_mm?` (its
  centre), `visual_style?` (`"shaded"` draws it as a picture).
- `drawing.view.projected`: `parent`, `side` (`left` | `right` | `above` | `below`, or a diagonal for
  a pictorial view).
- `drawing.view.section`: `parent`, `line: {kind: "full", points: [[x,y],[x,y]] (model mm on the
  parent's plane), arrows: "left" | "right"}`. Only full sections draw their cut so far.
- `drawing.view.auxiliary`: `parent`, `edge` or `face` (a model point on it): shows that face at its
  true shape.
- `drawing.view.flat`: a sheet-metal flat pattern.
- `drawing.view.inspect {"view": id}`: `direction`, `boundary_mm: {min, max}` on the sheet,
  `extents` in model mm, and its `dimensions`.
- `drawing.view.move {"view": id, "to_mm": [x, y]}`: views lined up with it follow.
- Detail views, break views and aligning views are not supported yet.

### Dimensions
`drawing.dimension.add`: `view`, `kind`, `decimals?` (default 2), `offset_mm?`, `text?`,
`override_text?`, `dry_run?`. Kinds:
- `{"linear":{"a": corner, "b": corner, "direction": "horizontal" | "vertical" | "aligned"}}`
- `{"length":{"edge": point}}`, `{"angle":{"a": edge, "b": edge}}`
- `{"radial":{"edge": a point on a circle or arc the view sees face on}}`: a whole circle reads Ø.
- `{"cylinder":{"face": point}}` (Ø of a cylinder seen side-on), `{"faces":{"a": point, "b": point}}`
  (two parallel faces seen edge-on), `{"chain":{"points":[3 to 64 corners]}}`.

A corner is a model point, or `{"centre": a point on a circle}` for a hole's centre. An edge or face
is a point on it. Points are matched to the model when the dimension is added and saved by the
edges' names, so the dimension follows the model when it changes. A point that hits nothing, or
several things, is refused.

The reply has `text` (what is printed), `value`, `status` and
`layout.dimension_lines: [[[x,y],[x,y]]]` in paper mm.

**Placing.** `offset_mm` is measured from the midpoint of the two measured points, not from the
part's outline, so a dimension from an edge to a hole lands inside the outline. Instead:
1. read the view's `boundary_mm` (`drawing.view.inspect`);
2. move each line with `drawing.dimension.edit {"dimension": id, "set": {"line_through_mm": [x, y]}}`:
   below the view at `y = min.y - 10` for feature positions and `min.y - 20` for the overall size;
   left of it at `x = min.x - 10`; and so on for the other sides;
3. `drawing.dimension.edit`'s `text_to_mm` moves only the text.

A dimension between two corners on the same outside edge already sits 10 mm outside it.

`drawing.dimension.from_model {"view": id}` brings in the model's own sketch dimensions (on a view
that sees the sketch face on) and extrude distances. A box's, cylinder's or hole's sizes come back in
`skipped` as `not_supported`: add those with `drawing.dimension.add`.

`drawing.dimension.inspect` reads one back; `drawing.dimension.repair` re-points a lost one.

### Notes, balloons and tables
- `drawing.note.add`: `anchor` (`{"sheet": name}` for a sheet note, `{"view": id}`, or
  `{"edge": {"view": id, "edge": point}}` for a leader), `at` (the note's **centre**, paper mm from
  the sheet's corner or from the view's centre), `content` (text, `\n` for new lines), `cap_mm?`.
  Text runs both ways from `at`: keep it 40 mm or more inside the border for a long line.
- `drawing.balloon.add`: `view`, `edge`, `at`; reads a parts list (`drawing.table.add {"kind":"parts_list"}`).
- `drawing.table.add`: `parts_list` (top right) or `revision` (bottom left).

### Checking and output
- `drawing.check {"sheet"?}`: `findings: [{kind, object, why}]` where `kind` is `lost`, `ambiguous`,
  `unmeasurable`, `suppressed`, `failed`, `overridden`, `out_of_date`, `nothing_shown`,
  `empty_field`, `not_ballooned` or `outside_sheet`; `coverage: [{feature, name, state}]` with
  `covered`, `not_covered`, `unknown` or `not_applicable`. A feature that is `not_covered` is drawn
  but has no dimension or note pointing at it.
  Name the `sheet` you are working on: a check of every sheet draws every view of every sheet,
  which takes minutes when one of them shows a whole assembly. Check the whole drawing once, last.
- `drawing.status`: each view's and dimension's state. Without the app's sheet page (headless, the
  CLI) views stay `out_of_date`; exports and `drawing.scene` still draw them whole.
- `drawing.scene {"sheet": name}`: everything on the sheet with where it sits (`objects[].bounds_mm`);
  large.
- `drawing.export`: `sheet` or `all_sheets: true`, `format` (`pdf` | `svg` | `png`), `path`. PDF has
  every sheet as a page at its real size, searchable text, and prints at actual size. PNG takes `dpi`.
  Writing a file waits for the user's approval.

## Traps
- `drawing.sheet.add` copies the paper of the sheet before it: after an A2 sheet a new one is A2
  unless `size` says otherwise. Read `size_mm` back before placing by coordinates.
- A note with a leader needs a point on a real edge: take an edge's `mid` from `model.edges`. A
  point on a face, or on a modelled thread's flank, is refused ("no edge at").
- A view of a whole large assembly at 1:1 can fail or run off the sheet; a detail sheet draws one
  part with `source`.
- A batch can't use an id made earlier in the same batch: see Ids.
- `offset_mm` from the midpoint: see Placing.
- `decimals` defaults to 2 (`100.00`); the built-in title block's general tolerance is ±0.2 mm, so
  whole millimetres read best with `decimals` 0.
- Dimensions on the isometric view are foreshortened; dimension the orthographic views.
- Dimensions of neighbouring views can meet in the gap between them; widen `gap_mm` or put a size on
  the other view that shows it (a plate's thickness on the right view, not the front).
- The model is locked while a drawing is open: `drawing.close` before changing the part, then
  `drawing.open` again. The drawing follows the change.
- `drawing.view.standard`'s `outside` lists views that don't fit: pick a smaller `scale` or a larger
  sheet (`drawing.sheet.set` `size`).

## Worked example
A 100 x 60 x 20 mm aluminium plate with a Ø10 through hole whose centre is 30 mm from the left and
front edges, on one A3 sheet.

**Batch 1:** the part, the drawing and its views.
```json
{"commands":[
 {"command":"solid.box","params":{"length":100,"width":60,"height":20,"body_name":"Plate"}},
 {"command":"solid.hole","params":{"position":[30,30,20],"diameter":10,"name":"Bore"}},
 {"command":"material.assign","params":{"bodies":["Plate"],"material":"Aluminum"}},
 {"command":"drawing.create","params":{"name":"Plate"}},
 {"command":"drawing.view.standard","params":{"sheet":"Sheet 1","gap_mm":35}}
]}
```
From the replies: `SOURCE` is step 4's `sources[0].id`, and `TOP` and `RIGHT` are step 5's `top` and
`right`. The scale fits at 1:1, and the top view's `boundary_mm` is
`{"min":[85.93,135.18],"max":[185.93,195.18]}`.

**Batch 2:** the title block, the dimensions and a note.
```json
{"commands":[
 {"command":"drawing.sheet.set","params":{"sheet":"Sheet 1","documents":{"source":"SOURCE","component":0}}},
 {"command":"drawing.properties.set","params":{"values":{"part_number":"PL-001","organization":"Workshop"}}},
 {"command":"drawing.revision.add","params":{"set":{"description":"First issue","designer":"KT","checker":"KT","date":"2026-11-19"}}},
 {"command":"drawing.dimension.add","params":{"view":"TOP","kind":{"linear":{"a":[0,0,20],"b":[100,0,20],"direction":"horizontal"}},"decimals":0}},
 {"command":"drawing.dimension.add","params":{"view":"TOP","kind":{"linear":{"a":[100,0,20],"b":[100,60,20],"direction":"vertical"}},"decimals":0}},
 {"command":"drawing.dimension.add","params":{"view":"TOP","kind":{"linear":{"a":[0,0,20],"b":{"centre":[35,30,20]},"direction":"horizontal"}},"decimals":0}},
 {"command":"drawing.dimension.add","params":{"view":"TOP","kind":{"linear":{"a":[0,0,20],"b":{"centre":[35,30,20]},"direction":"vertical"}},"decimals":0}},
 {"command":"drawing.dimension.add","params":{"view":"TOP","kind":{"radial":{"edge":[35,30,20]}},"decimals":0}},
 {"command":"drawing.dimension.add","params":{"view":"RIGHT","kind":{"linear":{"a":[100,60,0],"b":[100,60,20],"direction":"vertical"}},"decimals":0}},
 {"command":"drawing.note.add","params":{"anchor":{"sheet":"Sheet 1"},"at":[70,40],"content":"Break sharp edges 0.5 mm"}}
]}
```
The dimensions read `100`, `60`, `30`, `30`, `Ø10` and `20`. The 100 and 60 sit 10 mm outside the
outline. The two 30s sit inside it: the horizontal one at y = 140.18 and the vertical one at x = 90.93.
Name their ids from steps 4, 6 and 7: `D100`, `DX`, `DY`.

**Batch 3:** move the lines outside the top view: the 30s 10 mm out, the 100 behind them.
```json
{"commands":[
 {"command":"drawing.dimension.edit","params":{"dimension":"DX","set":{"line_through_mm":[100,125]}}},
 {"command":"drawing.dimension.edit","params":{"dimension":"D100","set":{"line_through_mm":[135,115]}}},
 {"command":"drawing.dimension.edit","params":{"dimension":"DY","set":{"line_through_mm":[76,150]}}},
 {"command":"drawing.check","params":{}}
]}
```
`drawing.check` then finds only the empty Finish cell, and its coverage counts both features
(`Box1`, `Bore`) as covered.
