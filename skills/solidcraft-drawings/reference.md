# SolidCraft drawings reference for agents

Command shapes for drawings, where things land on the sheet, the traps, and one worked example. The
example runs in CI (`drawings_skill_example_builds_as_documented` in `crates/mcp/src/tests.rs`), so
its numbers stay honest.

Paper positions are millimetres from the sheet's lower-left corner; model points are `[x, y, z]` in
the design's frame. `describe_command` gives any command's full docs; `list_commands {"prefix":"drawing."}`
lists them all.

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
- `drawing.status`: each view's and dimension's state. Without the app's sheet page (headless, the
  CLI) views stay `out_of_date`; exports and `drawing.scene` still draw them whole.
- `drawing.scene {"sheet": name}`: everything on the sheet with where it sits (`objects[].bounds_mm`);
  large.
- `drawing.export`: `sheet` or `all_sheets: true`, `format` (`pdf` | `svg` | `png`), `path`. PDF has
  every sheet as a page at its real size, searchable text, and prints at actual size. PNG takes `dpi`.
  Writing a file waits for the user's approval.

## Traps
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
