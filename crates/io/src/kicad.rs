//! KiCad schematics and boards, summarised for the assistant. A `.kicad_sch` or `.kicad_pcb` file
//! is megabytes of S-expressions; [`summary`] keeps what a person asks about: the parts with
//! their values and fields, the sheets and net labels of a schematic, and a board's nets, copper
//! layers, thickness and outline. Every byte is a stranger's: the text is read within limits on
//! size, nesting and count, and nothing here panics on any input.

use std::collections::{BTreeMap, BTreeSet, HashSet};

/// The most parts a summary lists; [`Summary::more`] counts the rest.
pub const MAX_PARTS: usize = 2000;
/// The most net labels a schematic's summary lists.
pub const MAX_LABELS: usize = 500;
/// The most net names a board's summary lists.
pub const MAX_NETS: usize = 2000;
/// The most hierarchical sheets a schematic's summary lists.
pub const MAX_SHEETS: usize = 500;
/// The largest file read, in bytes (64 MiB).
pub const MAX_KICAD_BYTES: usize = 64 * 1024 * 1024;
/// The most fields a part lists besides its reference, value and footprint.
const MAX_FIELDS: usize = 12;
/// The longest text kept, in characters; longer text is cut.
const MAX_TEXT: usize = 200;
/// The deepest nesting of lists read.
const MAX_DEPTH: usize = 256;
/// The most lists and atoms kept from one file.
const MAX_NODES: usize = 5_000_000;

/// The extensions of KiCad's own files.
const KICAD_EXTS: [&str; 7] = ["kicad_dru", "kicad_mod", "kicad_pcb", "kicad_pro", "kicad_sch", "kicad_sym", "kicad_wks"];

/// Is this a KiCad schematic, board, project, symbol library, footprint, design-rules or
/// drawing-sheet file, by extension (any case)?
pub fn is_kicad_path(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && KICAD_EXTS.iter().any(|k| ext.eq_ignore_ascii_case(k)))
}

/// What a KiCad schematic or board holds, small enough for the assistant to read.
#[derive(Debug, Clone, Default, serde::Serialize, PartialEq)]
pub struct Summary {
    /// "schematic" or "board".
    pub kind: String,
    /// The file format version, as written ("20231120").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The program that wrote the file ("eeschema", "pcbnew").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    /// The title block's title, date, rev, company and comments ("comment1"...), those present.
    pub title_block: BTreeMap<String, String>,
    /// A schematic's symbols or a board's footprints, in file order, at most [`MAX_PARTS`].
    pub parts: Vec<Part>,
    /// How many parts there are past [`MAX_PARTS`].
    pub more: usize,
    /// A schematic's hierarchical sheets.
    pub sheets: Vec<Sheet>,
    /// A schematic's net labels (local, global and hierarchical), each once, sorted, at most
    /// [`MAX_LABELS`].
    pub labels: Vec<String>,
    /// A board's net names, each once, sorted, at most [`MAX_NETS`].
    pub nets: Vec<String>,
    /// A board's copper layer count (the `*.Cu` layers of its layer table).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copper_layers: Option<usize>,
    /// A board's thickness in mm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thickness_mm: Option<f64>,
    /// The bounding box of a board's outline (its Edge.Cuts lines, rectangles, arcs, circles and
    /// polygons) in mm: `[min_x, min_y, max_x, max_y]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_mm: Option<[f64; 4]>,
}

/// One schematic symbol or board footprint.
#[derive(Debug, Clone, Default, serde::Serialize, PartialEq)]
pub struct Part {
    /// Its reference designator ("R1").
    pub reference: String,
    /// Its value ("10k").
    pub value: String,
    /// A symbol's library id ("Device:R"), or a footprint's name ("Resistor_SMD:R_0603_1608Metric").
    pub lib: String,
    /// A symbol's Footprint field; empty on a board.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub footprint: String,
    /// Its other fields with a value (MPN, Manufacturer, a datasheet's address...), at most 12,
    /// each value at most 200 characters.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
    /// Where it is placed: `[x, y, rotation in degrees]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 3]>,
    /// A footprint's side of the board: "top" or "bottom".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    /// Marked "do not populate".
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub dnp: bool,
    /// Left out of the bill of materials.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub not_in_bom: bool,
}

/// A schematic's hierarchical sheet: its name and the schematic file it shows.
#[derive(Debug, Clone, Default, serde::Serialize, PartialEq)]
pub struct Sheet {
    /// The sheet's name ("Power").
    pub name: String,
    /// The schematic file it shows ("power.kicad_sch").
    pub file: String,
}

/// A summary of a KiCad schematic (`.kicad_sch`) or board (`.kicad_pcb`, KiCad 5 or later), or
/// why the text isn't one.
pub fn summary(text: &str) -> Result<Summary, String> {
    if text.len() > MAX_KICAD_BYTES {
        return Err(format!("the file is {} MiB; at most {} MiB is read", text.len() / (1024 * 1024), MAX_KICAD_BYTES / (1024 * 1024)));
    }
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if text.is_empty() {
        return Err("the file is empty".into());
    }
    if text.starts_with("EESchema") {
        return Err("this is a KiCad 5 schematic (.sch), which is not read: open it in KiCad 6 or later and save it as .kicad_sch".into());
    }
    if text.starts_with('{') {
        return Err("this is JSON (a KiCad project or settings file?), not a KiCad schematic or board".into());
    }
    let root = parse(text)?;
    match root.head() {
        Some("kicad_sch") => Ok(schematic(&root)),
        Some("kicad_pcb") => Ok(board(&root)),
        Some("kicad_symbol_lib") => Err("this is a KiCad symbol library, not a schematic or board".into()),
        Some("footprint" | "module") => Err("this is a KiCad footprint, not a schematic or board".into()),
        Some("kicad_wks" | "page_layout") => Err("this is a KiCad drawing sheet, not a schematic or board".into()),
        _ => Err("this is not a KiCad schematic or board".into()),
    }
}

// ---------------------------------------------------------------------------------------------
// S-expressions

/// A list or an atom (a bare word or a quoted string, unquoted).
enum Node {
    Atom(String),
    List(Vec<Node>),
}

impl Node {
    /// A list's items; none for an atom.
    fn items(&self) -> &[Node] {
        match self {
            Node::List(items) => items,
            Node::Atom(_) => &[],
        }
    }

    fn atom(&self) -> Option<&str> {
        match self {
            Node::Atom(s) => Some(s),
            Node::List(_) => None,
        }
    }

    /// A list's first word: `kicad_pcb` in `(kicad_pcb ...)`.
    fn head(&self) -> Option<&str> {
        self.items().first()?.atom()
    }

    /// The `i`th item as an atom, counting the head as 0.
    fn arg(&self, i: usize) -> Option<&str> {
        self.items().get(i)?.atom()
    }

    /// The child lists whose head is `name`.
    fn lists<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.items().iter().filter(move |n| n.head() == Some(name))
    }

    /// The first child list whose head is `name`.
    fn child(&self, name: &str) -> Option<&Node> {
        self.items().iter().find(|n| n.head() == Some(name))
    }

    /// Does an atom after the head say `word`? (`(attr smd dnp)` says `dnp`.)
    fn has_word(&self, word: &str) -> bool {
        self.items().iter().skip(1).any(|n| n.atom() == Some(word))
    }
}

/// Lists under the top level that a summary never reads; skipping them keeps a large board's
/// tracks and zones out of memory.
const SKIP_TOP: &[&str] = &[
    "arc",
    "bus",
    "bus_entry",
    "dimension",
    "embedded_files",
    "embedded_fonts",
    "generated",
    "gr_text",
    "gr_text_box",
    "group",
    "image",
    "junction",
    "lib_symbols",
    "netclass_flag",
    "no_connect",
    "polyline",
    "rule_area",
    "segment",
    "setup",
    "sheet_instances",
    "table",
    "target",
    "text",
    "text_box",
    "via",
    "wire",
];

/// Lists at any depth below the top level that a summary never reads.
const SKIP_ANY: &[&str] = &[
    "effects",
    "embedded_files",
    "filled_polygon",
    "fill_segments",
    "fp_arc",
    "fp_circle",
    "fp_curve",
    "fp_line",
    "fp_poly",
    "fp_rect",
    "group",
    "model",
    "pad",
    "primitives",
    "zone",
];

enum Token {
    Open,
    Close,
    Atom(String),
}

/// Splits S-expression text into parentheses and atoms. `pos` is always at a character boundary:
/// it moves past ASCII bytes or whole characters only.
struct Lexer<'a> {
    text: &'a str,
    pos: usize,
}

impl Lexer<'_> {
    fn next_token(&mut self) -> Result<Option<Token>, String> {
        let bytes = self.text.as_bytes();
        while bytes.get(self.pos).is_some_and(|b| b.is_ascii_whitespace()) {
            self.pos += 1;
        }
        let Some(&b) = bytes.get(self.pos) else { return Ok(None) };
        match b {
            b'(' => {
                self.pos += 1;
                Ok(Some(Token::Open))
            }
            b')' => {
                self.pos += 1;
                Ok(Some(Token::Close))
            }
            b'"' => {
                self.pos += 1;
                self.quoted().map(|s| Some(Token::Atom(s)))
            }
            _ => {
                let start = self.pos;
                while bytes.get(self.pos).is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'(' | b')' | b'"')) {
                    self.pos += 1;
                }
                let word = self.text.get(start..self.pos).ok_or("the file is not valid text")?;
                Ok(Some(Token::Atom(word.to_string())))
            }
        }
    }

    /// The rest of a quoted string, its opening quote already read.
    fn quoted(&mut self) -> Result<String, String> {
        let mut out = String::new();
        loop {
            let rest = self.text.get(self.pos..).ok_or("the file is not valid text")?;
            let Some(stop) = rest.find(['"', '\\']) else { return Err("the file ends inside a quoted string".into()) };
            out.push_str(rest.get(..stop).ok_or("the file is not valid text")?);
            self.pos += stop;
            if rest.as_bytes().get(stop) == Some(&b'"') {
                self.pos += 1;
                return Ok(out);
            }
            // A backslash: the next character, escaped.
            self.pos += 1;
            let Some(c) = self.text.get(self.pos..).and_then(|r| r.chars().next()) else {
                return Err("the file ends inside a quoted string".into());
            };
            self.pos += c.len_utf8();
            out.push(match c {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                other => other,
            });
        }
    }
}

/// The file's first list, without the parts a summary never reads. Iterative, so nesting costs
/// no stack; at most [`MAX_DEPTH`] deep and [`MAX_NODES`] lists and atoms.
fn parse(text: &str) -> Result<Node, String> {
    let mut lexer = Lexer { text, pos: 0 };
    match lexer.next_token()? {
        Some(Token::Open) => {}
        _ => return Err("this is not a KiCad file: it does not start with '('".into()),
    }
    let mut stack: Vec<Vec<Node>> = vec![Vec::new()];
    let mut nodes = 1usize;
    // How deep inside a skipped list the lexer is; 0 when not skipping.
    let mut skipping = 0usize;
    loop {
        let Some(token) = lexer.next_token()? else {
            return Err("the file ends inside a list (unbalanced parentheses)".into());
        };
        if skipping > 0 {
            match token {
                Token::Open => {
                    skipping += 1;
                    if stack.len() + skipping > MAX_DEPTH {
                        return Err(format!("lists nest more than {MAX_DEPTH} deep"));
                    }
                }
                Token::Close => skipping -= 1,
                Token::Atom(_) => {}
            }
            continue;
        }
        match token {
            Token::Open => {
                if stack.len() >= MAX_DEPTH {
                    return Err(format!("lists nest more than {MAX_DEPTH} deep"));
                }
                nodes += 1;
                if nodes > MAX_NODES {
                    return Err(format!("the file has more than {MAX_NODES} items"));
                }
                stack.push(Vec::new());
            }
            Token::Close => {
                let Some(done) = stack.pop() else { return Err("unbalanced parentheses".into()) };
                let node = Node::List(done);
                match stack.last_mut() {
                    Some(parent) => parent.push(node),
                    None => return Ok(node),
                }
            }
            Token::Atom(word) => {
                nodes += 1;
                if nodes > MAX_NODES {
                    return Err(format!("the file has more than {MAX_NODES} items"));
                }
                let depth = stack.len();
                let Some(frame) = stack.last_mut() else { return Err("unbalanced parentheses".into()) };
                let skip =
                    frame.is_empty() && ((depth == 2 && SKIP_TOP.contains(&word.as_str())) || (depth >= 2 && SKIP_ANY.contains(&word.as_str())));
                if skip {
                    stack.pop();
                    skipping = 1;
                } else {
                    frame.push(Node::Atom(word));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Schematics and boards

/// `s`, at most [`MAX_TEXT`] characters.
fn cut(s: &str) -> String {
    s.chars().take(MAX_TEXT).collect()
}

/// A finite number.
fn num(s: Option<&str>) -> Option<f64> {
    s?.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// `(name x y)` under `node`.
fn point(node: &Node, name: &str) -> Option<(f64, f64)> {
    let p = node.child(name)?;
    Some((num(p.arg(1))?, num(p.arg(2))?))
}

/// `(at x y [rotation])` under `node`.
fn placement(node: &Node) -> Option<[f64; 3]> {
    let at = node.child("at")?;
    Some([num(at.arg(1))?, num(at.arg(2))?, num(at.arg(3)).unwrap_or(0.0)])
}

/// `(name yes)`, or a bare `(name)`.
fn says_yes(node: &Node, name: &str) -> bool {
    node.child(name).is_some_and(|n| matches!(n.arg(1), None | Some("yes" | "true")))
}

/// What both kinds of file share: version, generator and title block.
fn header(root: &Node, kind: &str) -> Summary {
    let mut title_block = BTreeMap::new();
    if let Some(tb) = root.child("title_block") {
        for key in ["title", "date", "rev", "company"] {
            if let Some(v) = tb.child(key).and_then(|n| n.arg(1)).filter(|v| !v.is_empty()) {
                title_block.insert(key.to_string(), cut(v));
            }
        }
        for c in tb.lists("comment").take(9) {
            if let (Some(n), Some(v)) = (c.arg(1), c.arg(2).filter(|v| !v.is_empty())) {
                title_block.insert(format!("comment{}", cut(n)), cut(v));
            }
        }
    }
    let generator = root.child("generator").or_else(|| root.child("host")).and_then(|n| n.arg(1)).map(cut);
    Summary { kind: kind.into(), version: root.child("version").and_then(|n| n.arg(1)).map(cut), generator, title_block, ..Summary::default() }
}

/// Fields a part's summary leaves out: the ones it shows elsewhere, and KiCad's bookkeeping.
fn is_shown_or_internal(key: &str) -> bool {
    matches!(key, "Reference" | "Value" | "Footprint" | "Sheetfile" | "Sheetname" | "Sheet file" | "Sheet name") || key.starts_with("ki_")
}

/// A part's reference, value and footprint from its `(property ...)` lists, and its other fields.
fn properties(node: &Node, part: &mut Part) {
    for p in node.lists("property") {
        let (Some(key), Some(value)) = (p.arg(1), p.arg(2)) else { continue };
        match key {
            "Reference" => part.reference = cut(value),
            "Value" => part.value = cut(value),
            "Footprint" => part.footprint = cut(value),
            _ if value.is_empty() || is_shown_or_internal(key) || (key == "Datasheet" && value == "~") => {}
            _ if part.fields.len() < MAX_FIELDS => {
                part.fields.insert(cut(key), cut(value));
            }
            _ => {}
        }
    }
}

/// Adds a part, or counts it past [`MAX_PARTS`].
fn add_part(out: &mut Summary, part: Part) {
    if out.parts.len() < MAX_PARTS {
        out.parts.push(part);
    } else {
        out.more += 1;
    }
}

fn schematic(root: &Node) -> Summary {
    let mut out = header(root, "schematic");
    // A symbol with several units (U1A, U1B) is placed once per unit: it is listed once.
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut labels = BTreeSet::new();
    for node in root.items().iter().skip(1) {
        match node.head() {
            Some("symbol") => {
                let mut part = Part { lib: node.child("lib_id").and_then(|n| n.arg(1)).map(cut).unwrap_or_default(), ..Part::default() };
                properties(node, &mut part);
                if (part.reference.is_empty() || part.reference.ends_with('?'))
                    && let Some(r) = instance_reference(node)
                {
                    part.reference = cut(r);
                }
                // Power symbols and flags ("#PWR01", "#FLG01") are not parts.
                if part.reference.starts_with('#') {
                    continue;
                }
                if !part.reference.is_empty() && !part.reference.ends_with('?') && !seen.insert((part.reference.clone(), part.lib.clone())) {
                    continue;
                }
                part.at = placement(node);
                part.dnp = says_yes(node, "dnp");
                part.not_in_bom = node.child("in_bom").is_some_and(|n| n.arg(1) == Some("no")) || says_yes(node, "exclude_from_bom");
                add_part(&mut out, part);
            }
            Some("sheet") if out.sheets.len() < MAX_SHEETS => {
                let mut sheet = Sheet::default();
                for p in node.lists("property") {
                    match (p.arg(1), p.arg(2)) {
                        (Some("Sheetname" | "Sheet name"), Some(v)) => sheet.name = cut(v),
                        (Some("Sheetfile" | "Sheet file"), Some(v)) => sheet.file = cut(v),
                        _ => {}
                    }
                }
                out.sheets.push(sheet);
            }
            Some("label" | "global_label" | "hierarchical_label") => {
                if let Some(text) = node.arg(1).filter(|t| !t.is_empty()) {
                    labels.insert(cut(text));
                }
            }
            _ => {}
        }
    }
    out.labels = labels.into_iter().take(MAX_LABELS).collect();
    out
}

/// The first reference in a symbol's `(instances (project ... (path ... (reference "R1"))))`.
fn instance_reference(symbol: &Node) -> Option<&str> {
    let instances = symbol.child("instances")?;
    instances.lists("project").flat_map(|p| p.lists("path")).find_map(|path| path.child("reference")?.arg(1)).filter(|r| !r.is_empty())
}

fn board(root: &Node) -> Summary {
    let mut out = header(root, "board");
    let mut nets = BTreeSet::new();
    let mut outline = Bounds::default();
    for node in root.items().iter().skip(1) {
        match node.head() {
            Some("footprint" | "module") => add_part(&mut out, footprint(node)),
            Some("net") => {
                // `(net 3 "GND")`; a later format may drop the number.
                let name = match node.items().len() {
                    2 => node.arg(1).filter(|n| n.parse::<u64>().is_err()),
                    _ => node.arg(2),
                };
                if let Some(name) = name.filter(|n| !n.is_empty()) {
                    nets.insert(cut(name));
                }
            }
            Some("general") => out.thickness_mm = num(node.child("thickness").and_then(|t| t.arg(1))).filter(|t| *t > 0.0),
            Some("layers") => {
                let copper = node.items().iter().skip(1).filter(|l| l.arg(1).is_some_and(|n| n.ends_with(".Cu"))).count();
                out.copper_layers = (copper > 0).then_some(copper);
            }
            Some(kind @ ("gr_line" | "gr_rect" | "gr_arc" | "gr_circle" | "gr_poly" | "gr_curve"))
                if node.child("layer").and_then(|l| l.arg(1)) == Some("Edge.Cuts") =>
            {
                edge(kind, node, &mut outline);
            }
            _ => {}
        }
    }
    out.nets = nets.into_iter().take(MAX_NETS).collect();
    out.outline_mm = outline.mm();
    out
}

/// A board footprint: KiCad 7 and later name its reference and value in `(property ...)`,
/// KiCad 5 and 6 in `(fp_text reference ...)` and `(fp_text value ...)`.
fn footprint(node: &Node) -> Part {
    let mut part = Part { lib: node.arg(1).map(cut).unwrap_or_default(), ..Part::default() };
    properties(node, &mut part);
    // A board names the footprint already: `lib`.
    part.footprint.clear();
    for t in node.lists("fp_text") {
        match (t.arg(1), t.arg(2)) {
            (Some("reference"), Some(r)) if part.reference.is_empty() => part.reference = cut(r),
            (Some("value"), Some(v)) if part.value.is_empty() => part.value = cut(v),
            _ => {}
        }
    }
    part.at = placement(node);
    part.side = match node.child("layer").and_then(|l| l.arg(1)) {
        Some("F.Cu") => Some("top".into()),
        Some("B.Cu") => Some("bottom".into()),
        _ => None,
    };
    if let Some(attr) = node.child("attr") {
        part.dnp = attr.has_word("dnp");
        part.not_in_bom = attr.has_word("exclude_from_bom");
    }
    part
}

/// A bounding box that grows point by point.
#[derive(Default)]
struct Bounds(Option<[f64; 4]>);

impl Bounds {
    fn add(&mut self, (x, y): (f64, f64)) {
        if !(x.is_finite() && y.is_finite()) {
            return;
        }
        self.0 = Some(match self.0 {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        });
    }

    /// The box, to 0.1 µm.
    fn mm(&self) -> Option<[f64; 4]> {
        self.0.map(|b| b.map(|v| if (v * 1e4).is_finite() { (v * 1e4).round() / 1e4 } else { v }))
    }
}

/// Grows `outline` by one Edge.Cuts graphic.
fn edge(kind: &str, node: &Node, outline: &mut Bounds) {
    match kind {
        "gr_line" | "gr_rect" => {
            for name in ["start", "end"] {
                if let Some(p) = point(node, name) {
                    outline.add(p);
                }
            }
        }
        "gr_circle" => {
            if let (Some((cx, cy)), Some((ex, ey))) = (point(node, "center"), point(node, "end")) {
                let r = (ex - cx).hypot(ey - cy);
                outline.add((cx - r, cy - r));
                outline.add((cx + r, cy + r));
            }
        }
        "gr_arc" => match (point(node, "start"), point(node, "mid"), point(node, "end")) {
            (Some(s), Some(m), Some(e)) => arc_bounds(s, m, e, outline),
            // KiCad 5: `start` is the centre, `end` where the arc starts, `angle` its sweep in degrees.
            (Some(c), None, Some(s)) => match num(node.child("angle").and_then(|a| a.arg(1))) {
                Some(angle) => {
                    let rotate = |deg: f64| {
                        let (sin, cos) = deg.to_radians().sin_cos();
                        let (dx, dy) = (s.0 - c.0, s.1 - c.1);
                        (c.0 + dx * cos - dy * sin, c.1 + dx * sin + dy * cos)
                    };
                    arc_bounds(s, rotate(angle / 2.0), rotate(angle), outline);
                }
                None => outline.add(s),
            },
            (s, m, e) => {
                for p in [s, m, e].into_iter().flatten() {
                    outline.add(p);
                }
            }
        },
        // A polygon's points, or a curve's (its control points hold the curve).
        _ => {
            if let Some(pts) = node.child("pts") {
                for p in pts.items().iter().skip(1) {
                    match p.head() {
                        Some("xy") => {
                            if let (Some(x), Some(y)) = (num(p.arg(1)), num(p.arg(2))) {
                                outline.add((x, y));
                            }
                        }
                        Some("arc") => {
                            if let (Some(s), Some(m), Some(e)) = (point(p, "start"), point(p, "mid"), point(p, "end")) {
                                arc_bounds(s, m, e, outline);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

/// Grows `outline` by the arc from `s` through `m` to `e`: its ends, and the points where it
/// is furthest left, right, up or down.
fn arc_bounds(s: (f64, f64), m: (f64, f64), e: (f64, f64), outline: &mut Bounds) {
    for p in [s, m, e] {
        outline.add(p);
    }
    // The centre: where the perpendicular bisectors of s-m and m-e meet.
    let (ax, ay, bx, by, cx, cy) = (s.0, s.1, m.0, m.1, e.0, e.1);
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    let scale = (bx - ax).abs().max((by - ay).abs()).max((cx - ax).abs()).max((cy - ay).abs());
    if !(d.abs() > 1e-12 * scale * scale) {
        return; // On one line: the three points are the arc.
    }
    let (a2, b2, c2) = (ax * ax + ay * ay, bx * bx + by * by, cx * cx + cy * cy);
    let ux = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / d;
    let uy = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / d;
    let r = (ax - ux).hypot(ay - uy);
    if !(ux.is_finite() && uy.is_finite() && r.is_finite()) {
        return;
    }
    let tau = std::f64::consts::TAU;
    let norm = |a: f64| a.rem_euclid(tau);
    let (a0, am, a1) = ((ay - uy).atan2(ax - ux), (by - uy).atan2(bx - ux), (cy - uy).atan2(cx - ux));
    // Measured counter-clockwise (in the file's axes) from `from`, the arc spans `span`.
    let (from, span) = if norm(am - a0) <= norm(a1 - a0) { (a0, norm(a1 - a0)) } else { (a1, norm(a0 - a1)) };
    for k in 0..4 {
        let theta = f64::from(k) * std::f64::consts::FRAC_PI_2;
        if norm(theta - from) <= span {
            outline.add((ux + r * theta.cos(), uy + r * theta.sin()));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const SCHEMATIC: &str = r##"(kicad_sch (version 20231120) (generator "eeschema") (generator_version "8.0")
  (uuid "6f1c2d3e-0000-4000-8000-000000000001")
  (paper "A4")
  (title_block (title "Blinky") (date "2024-01-02") (rev "B") (company "Acme \"Widgets\"") (comment 1 "First note"))
  (lib_symbols
    (symbol "Device:R" (pin_numbers hide)
      (property "Reference" "R" (at 2.032 0 90) (effects (font (size 1.27 1.27))))
      (property "Value" "R" (at 0 0 90))
      (symbol "R_0_1" (rectangle (start -1.016 -2.54) (end 1.016 2.54)))))
  (junction (at 120.65 50.8) (diameter 0) (color 0 0 0 0) (uuid "j1"))
  (wire (pts (xy 100 50) (xy 120 50)) (stroke (width 0) (type default)) (uuid "w1"))
  (label "SDA" (at 130 50.8 0) (effects (font (size 1.27 1.27))) (uuid "l1"))
  (global_label "VBUS" (shape input) (at 90 40 180) (uuid "l2"))
  (hierarchical_label "SCL" (shape bidirectional) (at 90 60 180) (uuid "l3"))
  (label "SDA" (at 140 50.8 0) (uuid "l4"))
  (symbol (lib_id "Device:R") (at 100.33 50.8 90) (unit 1) (in_bom yes) (on_board yes) (dnp no)
    (uuid "s1")
    (property "Reference" "R1" (at 0 0 0) (effects (font (size 1.27 1.27))))
    (property "Value" "10k" (at 0 0 0))
    (property "Footprint" "Resistor_SMD:R_0603_1608Metric" (at 0 0 0) (effects (hide yes)))
    (property "Datasheet" "~" (at 0 0 0))
    (property "Description" "" (at 0 0 0))
    (property "MPN" "RC0603FR-0710KL" (at 0 0 0))
    (pin "1" (uuid "p1")) (pin "2" (uuid "p2"))
    (instances (project "blinky" (path "/6f1c2d3e" (reference "R1") (unit 1)))))
  (symbol (lib_id "Device:R") (at 110 50.8 0) (unit 1) (in_bom no) (on_board yes) (dnp yes)
    (property "Reference" "R2" (at 0 0 0))
    (property "Value" "4k7" (at 0 0 0))
    (property "Footprint" "" (at 0 0 0))
    (property "Datasheet" "https://example.com/r.pdf" (at 0 0 0)))
  (symbol (lib_id "power:GND") (at 100 70 0) (unit 1)
    (property "Reference" "#PWR01" (at 0 0 0))
    (property "Value" "GND" (at 0 0 0)))
  (symbol (lib_id "Amplifier_Operational:LM358") (at 150 50 0) (unit 1)
    (property "Reference" "U1" (at 0 0 0)) (property "Value" "LM358" (at 0 0 0)))
  (symbol (lib_id "Amplifier_Operational:LM358") (at 150 80 0) (unit 2)
    (property "Reference" "U1" (at 0 0 0)) (property "Value" "LM358" (at 0 0 0)))
  (sheet (at 180 40) (size 20 15) (uuid "sh1")
    (property "Sheetname" "Power" (at 180 39 0))
    (property "Sheetfile" "power.kicad_sch" (at 180 56 0))
    (pin "VBUS" input (at 180 45 180) (uuid "sp1")))
  (sheet_instances (path "/" (page "1")))
)
"##;

    #[test]
    fn schematic_summary() {
        let s = summary(SCHEMATIC).unwrap();
        assert_eq!(s.kind, "schematic");
        assert_eq!(s.version.as_deref(), Some("20231120"));
        assert_eq!(s.generator.as_deref(), Some("eeschema"));
        assert_eq!(s.title_block.get("title").map(String::as_str), Some("Blinky"));
        assert_eq!(s.title_block.get("company").map(String::as_str), Some("Acme \"Widgets\""));
        assert_eq!(s.title_block.get("comment1").map(String::as_str), Some("First note"));
        let refs: Vec<&str> = s.parts.iter().map(|p| p.reference.as_str()).collect();
        assert_eq!(refs, ["R1", "R2", "U1"], "power symbol, library definition and second unit left out");
        let r1 = &s.parts[0];
        assert_eq!((r1.value.as_str(), r1.lib.as_str()), ("10k", "Device:R"));
        assert_eq!(r1.footprint, "Resistor_SMD:R_0603_1608Metric");
        assert_eq!(r1.fields.len(), 1);
        assert_eq!(r1.fields.get("MPN").map(String::as_str), Some("RC0603FR-0710KL"));
        assert_eq!(r1.at, Some([100.33, 50.8, 90.0]));
        assert!(!r1.dnp && !r1.not_in_bom && r1.side.is_none());
        let r2 = &s.parts[1];
        assert!(r2.dnp && r2.not_in_bom);
        assert_eq!(r2.fields.get("Datasheet").map(String::as_str), Some("https://example.com/r.pdf"));
        assert_eq!(s.sheets, [Sheet { name: "Power".into(), file: "power.kicad_sch".into() }]);
        assert_eq!(s.labels, ["SCL", "SDA", "VBUS"]);
        assert!(s.nets.is_empty() && s.copper_layers.is_none() && s.outline_mm.is_none());
        // Empty and default fields stay out of the JSON.
        let json = serde_json::to_value(&s).unwrap();
        assert!(json.get("thickness_mm").is_none());
        assert!(json["parts"][0].get("dnp").is_none());
        assert_eq!(json["parts"][1]["dnp"], true);
    }

    const BOARD8: &str = r#"(kicad_pcb (version 20240108) (generator "pcbnew") (generator_version "8.0")
  (general (thickness 1.6) (legacy_teardrops no))
  (paper "A4")
  (title_block (title "Blinky board") (rev "2"))
  (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (2 "In2.Cu" signal) (31 "B.Cu" signal)
    (32 "B.Adhes" user "B.Adhesive") (36 "B.SilkS" user "B.Silkscreen") (44 "Edge.Cuts" user))
  (setup (stackup (layer "F.SilkS" (type "Top Silk Screen"))) (pad_to_mask_clearance 0))
  (net 0 "") (net 1 "GND") (net 2 "/SDA") (net 3 "+3V3")
  (footprint "Resistor_SMD:R_0603_1608Metric" (layer "F.Cu") (uuid "f1") (at 120 80 90)
    (property "Reference" "R1" (at 0 -1.43 90) (layer "F.SilkS") (uuid "x") (effects (font (size 1 1) (thickness 0.15))))
    (property "Value" "10k" (at 0 1.43 90) (layer "F.Fab"))
    (property "Footprint" "Resistor_SMD:R_0603_1608Metric" (at 0 0 0) (layer "F.Fab") (hide yes))
    (property "Datasheet" "" (at 0 0 0) (layer "F.Fab"))
    (property "MPN" "RC0603FR-0710KL" (at 0 0 0) (layer "F.Fab"))
    (attr smd)
    (fp_line (start -0.8 -0.4) (end 0.8 -0.4) (stroke (width 0.1) (type solid)) (layer "F.Fab"))
    (pad "1" smd roundrect (at -0.8 0 90) (size 0.8 0.9) (layers "F.Cu" "F.Paste" "F.Mask") (net 1 "GND"))
    (model "${KICAD8_3DMODEL_DIR}/Resistor_SMD.3dshapes/R_0603_1608Metric.wrl" (offset (xyz 0 0 0))))
  (footprint "Capacitor_SMD:C_0402_1005Metric" (layer "B.Cu") (at 130 85 180)
    (property "Reference" "C1" (at 0 0 0)) (property "Value" "100n" (at 0 0 0)) (attr smd exclude_from_bom dnp))
  (gr_line (start 100 70) (end 150 70) (stroke (width 0.05) (type default)) (layer "Edge.Cuts") (uuid "e1"))
  (gr_line (start 100 100) (end 150 100) (layer "Edge.Cuts"))
  (gr_line (start 100 100) (end 100 70) (layer "Edge.Cuts"))
  (gr_arc (start 150 70) (mid 155.6125 94.0139) (end 150 100) (layer "Edge.Cuts"))
  (gr_line (start 200 200) (end 300 300) (layer "F.SilkS"))
  (segment (start 1 1) (end 2 2) (width 0.25) (layer "F.Cu") (net 1))
  (zone (net 1) (net_name "GND") (layer "F.Cu") (filled_polygon (layer "F.Cu") (pts (xy 0 0) (xy 1 1))))
)
"#;

    #[test]
    fn kicad8_board_summary() {
        let s = summary(BOARD8).unwrap();
        assert_eq!(s.kind, "board");
        assert_eq!(s.version.as_deref(), Some("20240108"));
        assert_eq!(s.generator.as_deref(), Some("pcbnew"));
        assert_eq!(s.title_block.get("rev").map(String::as_str), Some("2"));
        assert_eq!(s.copper_layers, Some(4));
        assert_eq!(s.thickness_mm, Some(1.6));
        assert_eq!(s.nets, ["+3V3", "/SDA", "GND"]);
        assert_eq!(s.parts.len(), 2);
        let r1 = &s.parts[0];
        assert_eq!((r1.reference.as_str(), r1.value.as_str()), ("R1", "10k"));
        assert_eq!(r1.lib, "Resistor_SMD:R_0603_1608Metric");
        assert!(r1.footprint.is_empty());
        assert_eq!(r1.fields.keys().collect::<Vec<_>>(), ["MPN"]);
        assert_eq!((r1.at, r1.side.as_deref()), (Some([120.0, 80.0, 90.0]), Some("top")));
        assert!(!r1.dnp && !r1.not_in_bom);
        let c1 = &s.parts[1];
        assert_eq!((c1.reference.as_str(), c1.side.as_deref()), ("C1", Some("bottom")));
        assert!(c1.dnp && c1.not_in_bom);
        // The arc bulges past its mid point to the circle's rightmost point, 140 + sqrt(325).
        let [x0, y0, x1, y1] = s.outline_mm.unwrap();
        assert_eq!([x0, y0, y1], [100.0, 70.0, 100.0]);
        assert!((x1 - (140.0 + 325f64.sqrt())).abs() < 1e-3, "{x1}");
    }

    #[test]
    fn kicad6_board_reads_fp_text() {
        let text = r#"(kicad_pcb (version 20211014) (generator pcbnew)
  (footprint "LED_SMD:LED_0805_2012Metric" (layer "F.Cu") (tedit 5F68FEF1) (tstamp "t1") (at 10 20)
    (property "Sheetfile" "blinky.kicad_sch") (property "Sheetname" "")
    (attr smd exclude_from_bom)
    (fp_text reference "D1" (at 0 -1.65) (layer "F.SilkS") (effects (font (size 1 1))))
    (fp_text value "Red" (at 0 1.65) (layer "F.Fab"))
    (fp_text user "${REFERENCE}" (at 0 0) (layer "F.Fab")))
  (gr_rect (start 0 0) (end 40 30) (layer "Edge.Cuts") (width 0.1)))"#;
        let s = summary(text).unwrap();
        let d1 = &s.parts[0];
        assert_eq!((d1.reference.as_str(), d1.value.as_str()), ("D1", "Red"));
        assert!(d1.fields.is_empty(), "{:?}", d1.fields);
        assert_eq!(d1.at, Some([10.0, 20.0, 0.0]));
        assert!(d1.not_in_bom && !d1.dnp);
        assert_eq!(s.outline_mm, Some([0.0, 0.0, 40.0, 30.0]));
    }

    #[test]
    fn kicad5_module_board_summary() {
        let text = "(kicad_pcb (version 20171130) (host pcbnew 5.1.9)
  (general (thickness 1.6) (drawings 4) (tracks 0) (zones 0) (modules 1) (nets 2))
  (page A4)
  (layers (0 F.Cu signal) (31 B.Cu signal) (44 Edge.Cuts user))
  (net 0 \"\") (net 1 GND)
  (module Resistor_SMD:R_0805_2012Metric (layer B.Cu) (tedit 5B36C52B) (tstamp 5C0E1234) (at 50.8 25.4 180)
    (descr \"Resistor SMD 0805\") (path /5C0E1111) (attr smd)
    (fp_text reference R7 (at 0 -1.65) (layer B.SilkS) (effects (font (size 1 1) (thickness 0.15)) (justify mirror)))
    (fp_text value 4k7 (at 0 1.65) (layer B.Fab))
    (pad 1 smd roundrect (at -0.9375 0 180) (size 0.975 1.4) (layers B.Cu B.Paste B.Mask) (net 1 GND)))
  (gr_circle (center 40 20) (end 45 20) (layer Edge.Cuts) (width 0.05))
  (gr_arc (start 60 20) (end 60 15) (angle 90) (layer Edge.Cuts) (width 0.05))
)";
        let s = summary(text).unwrap();
        assert_eq!(s.version.as_deref(), Some("20171130"));
        assert_eq!(s.generator.as_deref(), Some("pcbnew"));
        assert_eq!(s.copper_layers, Some(2));
        assert_eq!(s.thickness_mm, Some(1.6));
        assert_eq!(s.nets, ["GND"]);
        let r7 = &s.parts[0];
        assert_eq!((r7.reference.as_str(), r7.value.as_str()), ("R7", "4k7"));
        assert_eq!(r7.lib, "Resistor_SMD:R_0805_2012Metric");
        assert_eq!((r7.at, r7.side.as_deref()), (Some([50.8, 25.4, 180.0]), Some("bottom")));
        let [x0, y0, x1, y1] = s.outline_mm.unwrap();
        assert_eq!([x0, y0, y1], [35.0, 15.0, 25.0]);
        assert!(x1 >= 60.0, "{x1}");
    }

    #[test]
    fn kicad_paths() {
        for p in ["a.kicad_sch", "dir/B.KICAD_PCB", "x.kicad_pro", "lib.kicad_sym", "R.kicad_mod", "r.kicad_dru", "s.kicad_wks"] {
            assert!(is_kicad_path(p), "{p}");
        }
        for p in ["a.sch", "kicad_sch", ".kicad_sch", "a.kicad_sch.bak", "a.step", ""] {
            assert!(!is_kicad_path(p), "{p}");
        }
    }

    #[test]
    fn other_files_are_refused() {
        for (text, says) in [
            ("", "empty"),
            ("   \n", "empty"),
            ("EESchema Schematic File Version 4\n", "KiCad 5 schematic"),
            ("{\"board\": {}}", "JSON"),
            ("(kicad_symbol_lib (version 20231120))", "symbol library"),
            ("(footprint \"R\" (layer \"F.Cu\"))", "footprint"),
            ("(foo (bar baz))", "not a KiCad"),
            ("hello world", "does not start"),
            ("(kicad_sch (version 1)", "unbalanced"),
            ("(kicad_sch (title \"open", "quoted string"),
            ("(kicad_pcb (title \"x\\", "quoted string"),
        ] {
            let err = summary(text).unwrap_err();
            assert!(err.contains(says), "{text:?}: {err}");
        }
    }

    #[test]
    fn hostile_input_never_panics() {
        assert!(summary(&"(".repeat(10_000)).unwrap_err().contains("deep"));
        assert!(summary(&format!("(kicad_pcb (zone {}", "(".repeat(10_000))).unwrap_err().contains("deep"));
        // Random bytes, as text.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for len in [1usize, 7, 100, 5000] {
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    [b'(', b')', b'"', b'\\', b' ', b'a', 0xc3, 0xa9, 0xff][(seed % 9) as usize]
                })
                .collect();
            let _ = summary(&String::from_utf8_lossy(&bytes));
        }
        // Every prefix of real files.
        for text in [SCHEMATIC, BOARD8] {
            for (i, _) in text.char_indices() {
                let _ = summary(&text[..i]);
            }
        }
        // Numbers that aren't.
        let s = summary(
            "(kicad_pcb (general (thickness NaN)) (layers) (net) (net 1) (footprint) (at)
               (gr_line (start inf 0) (end 1e400 0) (layer Edge.Cuts))
               (gr_arc (start 0 0) (mid 0 0) (end 0 0) (layer Edge.Cuts))
               (gr_arc (start 1e308 -1e308) (mid -1e308 1e308) (end 1e308 1e308) (layer Edge.Cuts)))",
        )
        .unwrap();
        assert_eq!((s.thickness_mm, s.copper_layers), (None, None));
        assert!(s.nets.is_empty());
        assert_eq!(s.parts.len(), 1);
        assert!(s.outline_mm.is_some_and(|b| b.iter().all(|v| v.is_finite())));
        assert!(summary(&" ".repeat(MAX_KICAD_BYTES + 1)).unwrap_err().contains("MiB"));
    }

    #[test]
    fn long_text_and_many_parts_are_capped() {
        let long = "x".repeat(10_000);
        let mut text = String::from("(kicad_sch (version 20231120) (symbol (lib_id \"X\") (property \"Reference\" \"X1\")");
        for i in 0..20 {
            text.push_str(&format!("(property \"F{i}\" \"v\")"));
        }
        text.push(')');
        for i in 0..MAX_PARTS + 4 {
            text.push_str(&format!("(symbol (lib_id \"Device:R\") (property \"Reference\" \"R{i}\") (property \"Note\" \"{long}\"))"));
        }
        text.push(')');
        let s = summary(&text).unwrap();
        assert_eq!((s.parts.len(), s.more), (MAX_PARTS, 5));
        assert_eq!(s.parts[0].fields.len(), MAX_FIELDS);
        assert_eq!(s.parts[1].fields.get("Note").map(|v| v.chars().count()), Some(MAX_TEXT));
    }
}
