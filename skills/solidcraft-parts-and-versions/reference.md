# Parts libraries and versions: commands

Run these with `execute` (or several in one `batch`). None of them change the open design. Paths in
`versions.*` are relative to the folder, with `/` between folders; `folder` is a full path.

## Parts Library

- `execute library.parts.search {"folder":"/Users/me/ROV PARTS","query":"t200 thruster","limit":20}` lists
  parts best first: `path`, `file` (on this computer), `name`, `folder`, `kind` (`design` or `cad`),
  `format`, `bytes`, `part_number?`, `description?`, `revision?`; and `matched`, `total`, `truncated`.
  Every word of `query` must appear in the name, folders, file type, part number or description.
- `execute library.parts.convert {"path":"/Users/me/ROV PARTS/THRUSTERS/T200.STEP","stand_up":true}` saves
  `T200.solidcraft` beside the STEP file, with its picture and properties (`name?`, `part_number?`,
  `description?`, `organization?`). It refuses when that design exists, unless `overwrite: true`.
- Insert a library part into the open assembly, linked: `execute component.insert_design {"path":"<file>"}`.
  For a `cad` part, convert it first and insert the `.solidcraft` file it returns.
- `execute component.links {"check":true}` lists linked parts and which have `changed`;
  `execute component.update_links {"components":[3]}` brings in the new shape of those parts.

## Versions

| Command | Does | Reply |
|---|---|---|
| `versions.start` `{folder}` | starts keeping versions (nothing checked in yet) | as `versions.status` |
| `versions.status` `{folder}` | what a check-in would save | `tracked`, `head?`, `changes: [{path, change: added/changed/deleted, local}]`, `files` |
| `versions.check_in` `{folder, message, paths, author?, release?: {path, revision}}` | saves those files as a new version | `commit`, `short`, `files`, `revision?` |
| `versions.history` `{folder, path?, limit?}` | versions newest first (with `path`: only that part's) | `commits: [{id, short, message, author, time, files, versions, revisions}]`, `more` |
| `versions.restore` `{folder, commit, path?}` | puts one part (or every part) back as it was | `restored: [paths]` |
| `versions.copy` `{folder, commit, path, to?}` | an old version to a new file: `to` a new part in the folder, else a copy to open | `file`, `path?` |
| `versions.name` `{folder, commit, version?}` or `{folder, commit, path, revision}` | names a version ("v1.3") or releases a part revision ("Rev B") | `tag`, `commit` |

`time` is seconds since 1970 (UTC). A check-in with nothing changed, a message that is blank, a
path outside the folder, or a revision the part already has is refused with a reason to pass on.

## A worked session

The user saved `Frame plate.solidcraft` in `/Users/me/ROV/PROJECT` and asks you to check it in as Rev B:

1. `execute versions.status {"folder":"/Users/me/ROV/PROJECT"}`. If `tracked` is false, ask, then
   `execute versions.start {"folder":"/Users/me/ROV/PROJECT"}`.
2. Show them `changes` in words ("Frame plate changed, Bracket is new") and ask for the message.
3. `execute versions.check_in {"folder":"/Users/me/ROV/PROJECT","message":"Thicker plate","paths":["Frame plate.solidcraft"],"release":{"path":"Frame plate.solidcraft","revision":"Rev B"}}`.
   To show the revision in the part's own properties too, first set it with
   `execute document.metadata.set {"values":{"revision":"Rev B"}}` and save the part with `save {}`.
4. Tell them: "It's in the history now. Click the turn-back arrow at the top right of the Browser
   panel (Show Versions) to see it."

To go back: `versions.history` with `path`, pick the `id` by its message or revision, confirm with
them, then `versions.restore`. If that part is open with unsaved changes, ask them to save first.
