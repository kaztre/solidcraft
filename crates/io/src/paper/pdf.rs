//! A paper scene as a vector PDF (#4890, P1, P2): one page the sheet's size, its lines and fills,
//! its text and its pictures, in optional-content layers; several scenes as the pages of one
//! document ([`paper_pdf_pages`], [`paper_pdf_document`]), each its sheet's size, sharing one set
//! of layers and one copy of each font.
//!
//! - **The file** comes from `pdf-writer`: a catalog, one page whose MediaBox is the sheet in
//!   points, its content deflated, and an Info dictionary with the Producer, the Title when there
//!   is one (ISO 32000-1 14.3.3: leave out what isn't known) and dates only from
//!   [`PdfOptions::output_time`], never the clock, so a scene always gives the same bytes.
//! - **Numbers** are the scene on a 10 nm grid ([`PER_MM`] steps a millimetre, whose walk gives the
//!   joints, closures and merges SVG and PNG draw), written as millimetres with up to five
//!   decimals (Annex C's precision) inside one `cm` that scales millimetres to points. y is up on
//!   the page as in the scene, so nothing flips.
//! - **Curves:** lines `l` and cubics `c` as they are; a parabola raised to a cubic; an arc's
//!   pieces (each about a quarter turn, never a half) cut into cubics that stay within [`ARC_TOLERANCE_MM`],
//!   their rounding onto the grid included.
//! - **Paint:** strokes at their width `w` (the walk never gives 0, which 8.4.3.2 says is a device
//!   pixel and shouldn't be used), butt caps, miter joins and a miter limit of 4, dashed by `d`;
//!   fills black, even-odd (`f*`). Black on no background, as SVG.
//! - **Text** (P2) is real text: each font a Type 0 font whose glyphs are the embedded file's
//!   (a subset of the face, or the whole face where its licence says so: [`Embed`]), shown by
//!   glyph id at the places the scene laid them out (`TJ`, with a mark's rise `Ts`), with a
//!   ToUnicode CMap and the run's own text as its `ActualText`, so it is searched and copied as
//!   it reads. A face that may not be embedded, or can't be, has its glyphs' outlines filled
//!   (nonzero, as SVG fills them) with the run's text as their `ActualText`.
//! - **Pictures** are image XObjects of their pixels on their own grid at the page's picture
//!   resolution ([`PdfPage::pictures`]), RGB with their alpha as a soft mask, cut by their
//!   object's clip; a page given no pixels passes its pictures over, as P1 did.
//! - **Actual size:** [`PdfOptions::actual_size`] asks viewers to print the page at its size
//!   (`PrintScaling /None`), as a drawing is checked with a rule.
//! - **Layers:** nonempty groups share one PDF optional-content group for each
//!   distinct paper object kind. Group order remains the original paint order,
//!   and each kind's PDF layer uses ObjectKind::layer, as in DXF. Empty
//!   groups do not invent layers. Consecutive painted groups of the same kind
//!   share one marked-content run; empty groups do not interrupt it. All present
//!   layers start visible.
//!   Set [`PdfOptions::layers`] to false for the same vector marks without layers.
//! - **Content size:** every append checks the cumulative byte limit before growing the
//!   buffer, including inside a path/arc and the final graphics-state restore. Allocation
//!   failure is an error. The limit is for uncompressed content, not total export memory.

use std::collections::BTreeMap;
use std::f64::consts::FRAC_PI_2;
use std::io::Write as _;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use pdf_writer::types::{CidFontType, FontFlags as PdfFontFlags, SystemInfo, UnicodeCmap};
use pdf_writer::{Date, Filter, Name, Pdf, Rect, Ref, Str, TextStr};
use solidcore_record::{Seg2, Vec2};
use solidcraft_geom::paper::text::{Embed, Font, TextRun};
use solidcraft_geom::paper::{
    Clip, Grid, GridDashes, GridGroup, GridItem, GridPath, GridPoint, GridReport, Group, Item, ObjectKind, Picture, Pictures, Piece, Prepared,
};
use solidcraft_geom::pixel_grid::{GridWindow, PixelGrid};
use solidcraft_geom::prelude::*;

use super::TEN_NANOMETRES;
use super::flatten::{seg, vec};
use super::png::{PICTURE_DPI, ask};
use crate::{IoError, MAX_DRAWING_BYTES, Result};

/// How far an arc's cubics may stray from the arc, rounding onto the grid included.
pub const ARC_TOLERANCE_MM: f64 = 1e-4;
/// Points a millimetre.
const PT_PER_MM: f64 = 72.0 / 25.4;
/// The smallest page side, in points (Annex C).
const MIN_PAGE_PT: f64 = 3.0;
/// The most pages a sheet PDF may have: a drawing's sheets.
const MAX_PAGES: usize = solidcraft_doc::drawing::MAX_SHEETS;
/// The most characters a title may have.
const MAX_TITLE_CHARS: usize = 1000;
/// The last second a PDF date can hold: 9999-12-31 23:59:59 UTC.
const MAX_OUTPUT_TIME: u64 = 253_402_300_799;
/// Steps a millimetre on the grid: 10 nm steps, five decimals (Annex C's precision).
const PER_MM: f64 = TEN_NANOMETRES.per_mm();

/// The grid the scene is written on.
fn grid() -> Result<Grid> {
    TEN_NANOMETRES.grid()
}

/// What a sheet PDF says beyond the scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfOptions {
    /// The document's title, left out when `None` or empty.
    pub title: Option<String>,
    /// When the PDF was made, in seconds since 1970 (UTC): its creation and modification dates.
    /// Without it the file has no dates.
    pub output_time: Option<u64>,
    /// Include optional-content layers by object kind (default true). False writes
    /// the same vector marks in paint order, without layer objects or markers.
    pub layers: bool,
    /// Ask viewers to print each page at its size, not fitted to the printer's paper
    /// (`PrintScaling /None`; default false, which leaves it to the viewer).
    pub actual_size: bool,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self { title: None, output_time: None, layers: true, actual_size: false }
    }
}

/// One page of a sheet PDF: its scene, and where its pictures' pixels come from with the
/// resolution they are embedded at (25 to 1200 dpi); without them its pictures are passed over.
#[derive(Clone, Copy)]
pub struct PdfPage<'a, 'b> {
    pub scene: &'a Prepared<'b>,
    pub pictures: Option<(&'a dyn Pictures, f64)>,
}

/// What writing a scene did that the scene didn't say: the grid's report.
pub type PdfReport = GridReport;

/// A scene's PDF document, and what writing it did: [`paper_pdf_pages`] with one page.
pub fn paper_pdf(scene: &Prepared, options: &PdfOptions) -> Result<(Vec<u8>, PdfReport)> {
    let (bytes, mut reports) = paper_pdf_pages(std::slice::from_ref(scene), options)?;
    let report = reports.pop().ok_or_else(|| IoError::Invalid("the sheet PDF has no page".into()))?;
    Ok((bytes, report))
}

/// Scenes as the pages of one PDF document, in order, each page its scene's size, and what
/// writing each page did: [`paper_pdf_document`] with no pictures' pixels.
pub fn paper_pdf_pages(scenes: &[Prepared], options: &PdfOptions) -> Result<(Vec<u8>, Vec<PdfReport>)> {
    let pages: Vec<PdfPage> = scenes.iter().map(|scene| PdfPage { scene, pictures: None }).collect();
    paper_pdf_document(&pages, options)
}

/// Pages as one PDF document, in order, each page its scene's size, and what writing each page
/// did. The pages share one allowance of content, [`MAX_DRAWING_BYTES`], and another of pictures.
pub fn paper_pdf_document(scenes: &[PdfPage], options: &PdfOptions) -> Result<(Vec<u8>, Vec<PdfReport>)> {
    if let Some(dpi) = scenes.iter().find_map(|page| page.pictures.map(|(_, dpi)| dpi)).filter(|dpi| !PICTURE_DPI.contains(dpi)) {
        return Err(IoError::Invalid(format!("a PDF's pictures are 25 to 1200 dpi, not {dpi}")));
    }
    let title = options.title.as_deref().filter(|t| !t.is_empty());
    if title.is_some_and(|t| t.chars().count() > MAX_TITLE_CHARS) {
        return Err(IoError::Invalid(format!("a sheet PDF's title is at most {MAX_TITLE_CHARS} characters")));
    }
    if !(1..=MAX_PAGES).contains(&scenes.len()) {
        return Err(IoError::Invalid(format!("a sheet PDF has 1 to {MAX_PAGES} pages")));
    }
    let date = options.output_time.map(date).transpose()?;
    let grid = grid()?;
    // The pages' sizes from the scenes, as the grid will make them. Each page is put on the grid
    // only when it is written, so one page's grid is held at a time.
    let mut sizes = Vec::with_capacity(scenes.len());
    for page in scenes {
        let size = grid.point(page.scene.size_mm()).map_err(|e| IoError::Invalid(e.to_string()))?;
        let page = [size.x, size.y].map(|n| n as f64 / PER_MM * PT_PER_MM);
        if page.iter().any(|side| *side < MIN_PAGE_PT) {
            return Err(IoError::Invalid(format!("a sheet PDF's page is at least {MIN_PAGE_PT} pt (about 1.06 mm) a side")));
        }
        sizes.push(page);
    }

    // Object numbers in the order the objects are written: the catalog, the page tree, each page
    // and its content, the Info dictionary, then the layers. One page keeps 1 to 5 and layers
    // from 6.
    let mut refs = Refs(1);
    let (catalog, tree) = (refs.next()?, refs.next()?);
    let mut pages = Vec::with_capacity(sizes.len());
    for _ in &sizes {
        pages.push((refs.next()?, refs.next()?));
    }
    let info = refs.next()?;
    let painted_kinds = scenes.iter().flat_map(|page| page.scene.groups().iter().filter(move |group| painted(group, page.pictures.is_some())));
    let layers = if options.layers { collect_layers(painted_kinds.map(|group| group.kind), &mut refs)? } else { Vec::new() };
    // Then the fonts, each once for the document, and the pictures, numbered as they are drawn.
    let uses = font_uses(scenes.iter().map(|page| page.scene))?;
    let fonts = pdf_fonts(&uses, &mut refs)?;

    let mut pdf = Pdf::new();
    {
        let mut root = pdf.catalog(catalog);
        root.pages(tree);
        if !layers.is_empty() {
            // PDF 1.5 optional content: one group per used paper object kind.
            // The default configuration orders all layers and starts them ON.
            let mut properties = root.insert(Name(b"OCProperties")).dict();
            properties.insert(Name(b"OCGs")).array().items(layers.iter().map(|l| l.reference));
            let mut default = properties.insert(Name(b"D")).dict();
            default.insert(Name(b"Order")).array().items(layers.iter().map(|l| l.reference));
            default.insert(Name(b"ON")).array().items(layers.iter().map(|l| l.reference));
            default.pair(Name(b"BaseState"), Name(b"ON"));
        }
        if options.actual_size {
            root.insert(Name(b"ViewerPreferences")).dict().pair(Name(b"PrintScaling"), Name(b"None"));
        }
    }
    let count = i32::try_from(pages.len()).map_err(|_| IoError::Invalid("too many PDF pages".into()))?;
    pdf.pages(tree).kids(pages.iter().map(|(page, _)| *page)).count(count);
    let (mut used, mut reports) = (0usize, Vec::with_capacity(scenes.len()));
    let mut pictures_used = 0usize;
    let mut picture_count = 0usize;
    for ((sheet, page), (page_id, contents)) in scenes.iter().zip(sizes).zip(pages) {
        let scene = sheet.scene;
        let with_pictures = sheet.pictures.is_some();
        // This page alone on the grid, written, then dropped. The grid keeps one group for each of
        // the scene's, in order, so the scene says which of them are painted.
        let on = scene.on_grid(grid).map_err(|e| IoError::Invalid(e.to_string()))?;
        if on.groups.len() != scene.groups().len() {
            return Err(IoError::Invalid("the sheet PDF: a page's groups changed on the grid".into()));
        }
        let groups = scene.groups().iter().zip(&on.groups).filter(|(group, _)| painted(group, with_pictures)).map(|(group, on)| (Some(group), on));
        let page_fonts = PageFonts { scene, fonts: &fonts, uses: &uses };
        let drawn = page_content(
            groups,
            &page_fonts,
            sheet.pictures,
            options.layers.then_some(layers.as_slice()),
            MAX_DRAWING_BYTES - used,
            &mut picture_count,
        )?;
        used += drawn.content.len();
        let compressed = deflate(drawn.content.as_bytes())?;
        // Each picture's pixels, numbered after everything the page names before them.
        let mut images = Vec::with_capacity(drawn.pictures.len());
        for planned in drawn.pictures {
            images.push((planned.name.clone(), refs.next()?, planned));
        }
        let mut writer = pdf.page(page_id);
        writer.parent(tree).media_box(Rect::new(0.0, 0.0, page[0] as f32, page[1] as f32)).contents(contents);
        {
            // This page's layers (the kinds it paints), fonts and pictures.
            let mut resources = writer.resources();
            let mut on_page =
                layers.iter().filter(|layer| scene.groups().iter().any(|g| painted(g, with_pictures) && g.kind == layer.kind)).peekable();
            if on_page.peek().is_some() {
                let mut properties = resources.insert(Name(b"Properties")).dict();
                for layer in on_page {
                    properties.pair(Name(layer.tag.as_bytes()), layer.reference);
                }
            }
            let mut page_embedded = fonts.iter().filter(|f| drawn.fonts.contains(&f.id)).filter_map(|f| match &f.carried {
                Carried::Embedded(e) => Some((f.tag.as_str(), e.type0)),
                Carried::Outlines => None,
            });
            if let Some(first) = page_embedded.next() {
                let mut dict = resources.fonts();
                for (tag, reference) in std::iter::once(first).chain(page_embedded) {
                    dict.pair(Name(tag.as_bytes()), reference);
                }
            }
            if !images.is_empty() {
                let mut dict = resources.x_objects();
                for (name, reference, _) in &images {
                    dict.pair(Name(name.as_bytes()), *reference);
                }
            }
        }
        drop(writer);
        pdf.stream(contents, &compressed).filter(Filter::FlateDecode);
        if let Some((pictures, _)) = sheet.pictures {
            for (_, reference, planned) in images {
                pictures_used += write_picture(&mut pdf, reference, &mut refs, &planned, pictures, MAX_DRAWING_BYTES.saturating_sub(pictures_used))?;
            }
        }
        reports.push(on.report);
    }
    write_fonts(&mut pdf, &fonts, &uses)?;
    let mut doc = pdf.document_info(info);
    doc.producer(TextStr(concat!("SolidCraft ", env!("CARGO_PKG_VERSION"))));
    if let Some(title) = title {
        doc.title(TextStr(title));
    }
    if let Some(date) = date {
        doc.creation_date(date).modified_date(date);
    }
    drop(doc);
    for layer in &layers {
        let mut group = pdf.indirect(layer.reference).dict();
        group.pair(Name(b"Type"), Name(b"OCG"));
        group.pair(Name(b"Name"), TextStr(layer.kind.layer()));
    }
    Ok((pdf.finish(), reports))
}

/// The next object numbers of one document, each handed out once (pdf-writer panics on a number
/// written twice).
struct Refs(i32);

impl Refs {
    fn next(&mut self) -> Result<Ref> {
        let next = self.0.checked_add(1).ok_or_else(|| IoError::Invalid("too many PDF objects".into()))?;
        Ok(Ref::new(std::mem::replace(&mut self.0, next)))
    }
}

/// `secs` since 1970 as a PDF date in UTC, refused past the year 9999.
fn date(secs: u64) -> Result<Date> {
    if secs > MAX_OUTPUT_TIME {
        return Err(IoError::Invalid("a sheet PDF's date is at most the year 9999".into()));
    }
    let (year, month, day, hour, minute, second) = crate::civil(secs);
    let (month, day) = (u8::try_from(month).unwrap_or(1), u8::try_from(day).unwrap_or(1));
    let [hour, minute, second] = [hour, minute, second].map(|v| u8::try_from(v).unwrap_or(0));
    Ok(Date::new(u16::try_from(year).unwrap_or(0)).month(month).day(day).hour(hour).minute(minute).second(second).utc_offset_hour(0))
}

/// One PDF layer for every kind with actual paper marks, in first paint order.
struct Layer {
    kind: ObjectKind,
    reference: Ref,
    tag: String,
}

/// The layers of the painted groups' `kinds` (every page's, in page order): one a kind, numbered
/// from `refs` and tagged `L0`, `L1`… as the kinds are first painted, so a kind is the same layer on
/// every page.
fn collect_layers(kinds: impl IntoIterator<Item = ObjectKind>, refs: &mut Refs) -> Result<Vec<Layer>> {
    let mut layers: Vec<Layer> = Vec::new();
    for kind in kinds {
        if layers.iter().any(|layer| layer.kind == kind) {
            continue;
        }
        layers.try_reserve(1).map_err(|_| IoError::Invalid("the sheet PDF: cannot allocate its layer list".into()))?;
        layers.push(Layer { kind, reference: refs.next()?, tag: format!("L{}", layers.len()) });
    }
    Ok(layers)
}

/// Whether `group` puts any mark on this PDF's page: a stroke, a fill or text, and a picture
/// `with_pictures` (a page given no pixels passes its pictures over). A group with none invents no
/// layer and doesn't interrupt a run. Asked of the prepared scene, whose items each stay one on
/// the grid.
fn painted(group: &Group, with_pictures: bool) -> bool {
    group.items.iter().any(|item| match item {
        Item::Stroke { .. } | Item::Fill { .. } | Item::Text(_) => true,
        Item::Picture(_) => with_pictures,
    })
}

/// The page's content stream: the scene in millimetres, scaled to points once.
#[cfg(test)]
fn content(groups: &[GridGroup]) -> Result<String> {
    content_with_limit(groups, MAX_DRAWING_BYTES)
}

// The private seam permits smaller test limits, never a higher production cap. The tests give it
// painted groups only.
#[cfg(test)]
fn content_with_limit(groups: &[GridGroup], max_bytes: usize) -> Result<String> {
    // Numbered as a one-page document's, from 6.
    let layers = collect_layers(groups.iter().map(|group| group.kind), &mut Refs(6))?;
    content_with_limit_and_layers(groups, Some(&layers), max_bytes)
}

/// The content of a page's painted groups ([`painted`]), in paint order, with no scene behind it:
/// text is filled as its outlines, pictures passed over.
#[cfg(test)]
fn content_with_limit_and_layers<'a>(groups: impl IntoIterator<Item = &'a GridGroup>, layers: Option<&[Layer]>, max_bytes: usize) -> Result<String> {
    let none = Vec::new();
    let fonts = NoFonts(&none);
    Ok(page_content(groups.into_iter().map(|g| (None, g)), &fonts, None, layers, max_bytes, &mut 0)?.content)
}

/// What a page's content draws beyond its paths: the fonts it shows text in and the pictures it
/// places, each named as the content names it.
struct Drawn {
    content: String,
    fonts: std::collections::BTreeSet<[u8; 32]>,
    pictures: Vec<PlannedPicture>,
}

/// A picture the content places: the XObject it names, and the pixels it holds.
struct PlannedPicture {
    name: String,
    picture: Picture,
    grid: PixelGrid,
    window: GridWindow,
}

/// The fonts a page's text is set in.
trait TextFonts {
    /// Run `item` of `group`, and how its font is carried; `None` when there is no scene behind
    /// the content (its outlines are filled then).
    fn run<'s>(&'s self, group: Option<&'s Group>, item: usize) -> Option<(&'s TextRun, &'s PdfFont, &'s FontUse<'s>)>;
}

struct PageFonts<'a, 'b> {
    scene: &'a Prepared<'b>,
    fonts: &'a [PdfFont],
    uses: &'a BTreeMap<[u8; 32], FontUse<'a>>,
}

impl TextFonts for PageFonts<'_, '_> {
    fn run<'s>(&'s self, group: Option<&'s Group>, item: usize) -> Option<(&'s TextRun, &'s PdfFont, &'s FontUse<'s>)> {
        let Item::Text(run) = group?.items.get(item)? else { return None };
        let font = self.scene.font(run)?;
        let pdf = self.fonts.iter().find(|f| f.id == font.id)?;
        Some((run, pdf, self.uses.get(&font.id)?))
    }
}

/// No scene: text is filled as its outlines.
#[cfg(test)]
struct NoFonts<'a>(&'a Vec<PdfFont>);

#[cfg(test)]
impl TextFonts for NoFonts<'_> {
    fn run<'s>(&'s self, _: Option<&'s Group>, _: usize) -> Option<(&'s TextRun, &'s PdfFont, &'s FontUse<'s>)> {
        let _ = self.0;
        None
    }
}

/// The content of a page's painted groups, each with the prepared group behind it (for its text
/// runs and pictures), in paint order. Pictures are drawn only `with` their pixels; each gets the
/// next of `pictures`' numbers as its name.
fn page_content<'a>(
    groups: impl IntoIterator<Item = (Option<&'a Group>, &'a GridGroup)>,
    fonts: &dyn TextFonts,
    with: Option<(&dyn Pictures, f64)>,
    layers: Option<&[Layer]>,
    max_bytes: usize,
    pictures: &mut usize,
) -> Result<Drawn> {
    let mut drawn = Drawn { content: String::new(), fonts: Default::default(), pictures: Vec::new() };
    let mut w = Content::new(max_bytes)?;
    w.append(&format!("q {PT_PER_MM:.9} 0 0 {PT_PER_MM:.9} 0 0 cm 0 J 0 j 4 M\n"))?;
    let mut active_kind = None;
    for (prepared, group) in groups {
        if let Some(layers) = layers {
            let layer = layers.iter().find(|layer| layer.kind == group.kind).ok_or_else(|| IoError::Invalid("missing paper PDF layer".into()))?;
            if active_kind != Some(group.kind) {
                if active_kind.is_some() {
                    w.append("EMC\n")?;
                }
                w.append("/OC /")?;
                w.append(&layer.tag)?;
                w.append(" BDC\n")?;
                active_kind = Some(group.kind);
            }
        }
        for item in &group.items {
            match item {
                GridItem::Stroke { width, path, dashes } => {
                    if w.width != Some(*width) {
                        w.number(*width)?;
                        w.append(" w\n")?;
                        w.width = Some(*width);
                    }
                    if w.dashes.as_ref() != dashes.as_ref() {
                        w.dashes(dashes.as_ref())?;
                        w.dashes.clone_from(dashes);
                    }
                    w.path(path)?;
                    // A closed path's closing line, then the stroke: `s`.
                    w.append(if path.closed { "s\n" } else { "S\n" })?;
                }
                GridItem::Fill { loops } => {
                    for l in loops {
                        w.path(l)?;
                        w.append("h\n")?;
                    }
                    w.append("f*\n")?;
                }
                GridItem::Text { item, loops } => match fonts.run(prepared, *item) {
                    Some((run, font, used)) => {
                        w.run(run, font, used, loops)?;
                        drawn.fonts.insert(font.id);
                    }
                    None => w.outlines(loops)?,
                },
                GridItem::Picture { picture, clip, visible } => {
                    let Some((_, dpi)) = with else { continue };
                    let source = picture.source;
                    let grid = picture.grid(dpi).map_err(|e| IoError::Invalid(format!("picture {source}: {e}")))?;
                    let Some(window) = grid.window(*visible) else { continue };
                    let name = format!("Im{pictures}");
                    *pictures += 1;
                    w.picture(&name, &grid, window, clip.as_ref())?;
                    drawn.pictures.push(PlannedPicture { name, picture: *picture, grid, window });
                }
            }
        }
    }
    if active_kind.is_some() {
        w.append("EMC\n")?;
    }
    w.append("Q\n")?;
    drawn.content = w.out;
    Ok(drawn)
}

// One growth path for every operand/operator. Reserve geometrically, capped at
// the allowance, rather than repeatedly reallocating for individual numbers.
fn append_bounded(out: &mut String, text: &str, max_bytes: usize) -> Result<()> {
    let length = out
        .len()
        .checked_add(text.len())
        .filter(|&n| n <= max_bytes)
        .ok_or_else(|| IoError::Invalid(format!("the PDF's drawing would be over {max_bytes} bytes")))?;
    if length > out.capacity() {
        let capacity = out.capacity().saturating_mul(2).max(length).min(max_bytes);
        out.try_reserve_exact(capacity - out.len())
            .map_err(|_| IoError::Invalid("the sheet PDF: cannot allocate its bounded content buffer".into()))?;
    }
    out.push_str(text);
    Ok(())
}

struct Content {
    out: String,
    max_bytes: usize,
    // Reuse a small buffer for the existing shared integer formatter. Any i64
    // on this grid needs at most 21 bytes, including a minus sign and point.
    number_text: String,
    /// The stroke width and dash pattern last set, so each is written only when it changes.
    width: Option<i64>,
    dashes: Option<GridDashes>,
}

impl Content {
    fn new(max_bytes: usize) -> Result<Self> {
        let mut number_text = String::new();
        number_text.try_reserve_exact(32).map_err(|_| IoError::Invalid("the sheet PDF: cannot allocate its number buffer".into()))?;
        Ok(Self { out: String::new(), max_bytes: max_bytes.min(MAX_DRAWING_BYTES), number_text, width: None, dashes: None })
    }

    fn append(&mut self, text: &str) -> Result<()> {
        append_bounded(&mut self.out, text, self.max_bytes)
    }

    fn number(&mut self, n: i64) -> Result<()> {
        self.number_text.clear();
        TEN_NANOMETRES.push(&mut self.number_text, n);
        append_bounded(&mut self.out, &self.number_text, self.max_bytes)
    }

    fn point(&mut self, p: GridPoint) -> Result<()> {
        self.number(p.x)?;
        self.append(" ")?;
        self.number(p.y)?;
        self.append(" ")
    }

    /// `p` in steps, rounded onto the grid.
    fn near(&mut self, p: Vec2) -> Result<()> {
        if !p.is_finite() {
            return Err(IoError::Invalid("the sheet PDF: a curve point is not a number".into()));
        }
        self.point(GridPoint { x: p.x.round() as i64, y: p.y.round() as i64 })
    }

    /// A dash pattern `[on off ...] phase d`, or a solid line `[] 0 d`.
    fn dashes(&mut self, dashes: Option<&GridDashes>) -> Result<()> {
        self.append("[")?;
        if let Some(d) = dashes {
            for (i, length) in d.lengths.iter().enumerate() {
                if i > 0 {
                    self.append(" ")?;
                }
                self.number(*length)?;
            }
        }
        self.append("] ")?;
        self.number(dashes.map_or(0, |d| d.phase))?;
        self.append(" d\n")
    }

    /// Glyph outlines, filled nonzero as SVG fills them: text with no font behind it.
    fn outlines(&mut self, loops: &[GridPath]) -> Result<()> {
        for l in loops {
            self.path(l)?;
            self.append("h\n")?;
        }
        self.append("f\n")
    }

    /// A run of text, marked with its own text as `ActualText`: shown in its embedded font by glyph
    /// id, or (a font the PDF doesn't carry, or a glyph it lacks) as its glyphs' outlines `loops`.
    fn run(&mut self, run: &TextRun, font: &PdfFont, used: &FontUse<'_>, loops: &[GridPath]) -> Result<()> {
        self.append("/Span <</ActualText <")?;
        self.append(&utf16_hex(&run.text))?;
        self.append(">>> BDC\n")?;
        match &font.carried {
            Carried::Embedded(e) if run.glyphs.iter().all(|g| e.cids.contains_key(&g.id)) => self.shown(run, &font.tag, e, used)?,
            _ => self.outlines(loops)?,
        }
        self.append("EMC\n")
    }

    /// A run shown in its embedded font: one text matrix for the run (its em, angle and lean, at
    /// its start), each glyph moved to its place along the baseline with `TJ`'s adjustments, and
    /// up by its rise with `Ts`. The pen follows the widths `W` gives, so each glyph lands where
    /// the scene put it.
    fn shown(&mut self, run: &TextRun, tag: &str, e: &EmbeddedFont, used: &FontUse<'_>) -> Result<()> {
        let em = run.scale * f64::from(used.font.units_per_em);
        if !(em.is_finite() && em > 0.0) {
            return Err(IoError::Invalid("the sheet PDF: a run's size is not a number".into()));
        }
        let (sin, cos) = run.angle.sin_cos();
        let m = [em * cos, em * sin, em * (run.slant * cos - sin), em * (run.slant * sin + cos), run.origin.x, run.origin.y];
        self.append("BT\n/")?;
        self.append(tag)?;
        self.append(" 1 Tf\n")?;
        for v in m {
            self.append(&real(v, 6))?;
            self.append(" ")?;
        }
        self.append("Tm\n")?;
        let (mut pen, mut rise, mut open) = (0.0f64, 0.0f64, false);
        for g in &run.glyphs {
            let Some(cid) = e.cids.get(&g.id) else { continue };
            let up = g.rise / em;
            if (up - rise).abs() > 1e-9 {
                if open {
                    self.append("] TJ\n")?;
                    open = false;
                }
                self.append(&real(up, 6))?;
                self.append(" Ts\n")?;
                rise = up;
            }
            if !open {
                self.append("[")?;
                open = true;
            }
            // Where this glyph's origin goes, in text space: a lean moves a raised glyph along.
            let at = g.x / em - run.slant * rise;
            let shift = -(at - pen) * 1000.0;
            if shift.abs() >= 5e-4 {
                self.append(&real(shift, 3))?;
            }
            self.append(&format!("<{cid:04X}>"))?;
            pen = at + e.widths.get(cid).copied().unwrap_or(0.0) / 1000.0;
        }
        if open {
            self.append("] TJ\n")?;
        }
        self.append("ET\n")
    }

    /// Picture `name`'s pixels, `window` of its `grid`, placed on their pixel edges (on the
    /// content's grid, so they register with the lines) and cut by `clip`.
    fn picture(&mut self, name: &str, pixels: &PixelGrid, window: GridWindow, clip: Option<&Clip>) -> Result<()> {
        let g = grid()?;
        let on = |p: Vec2| g.point(p).map_err(|e| IoError::Invalid(e.to_string()));
        let low = on(Vec2::new(pixels.column(window.x0).0, pixels.row(window.y1.saturating_sub(1)).0))?;
        let high = on(Vec2::new(pixels.column(window.x1.saturating_sub(1)).1, pixels.row(window.y0).1))?;
        self.append("q\n")?;
        match clip {
            Some(Clip::Rect { min, max }) => {
                let (a, b) = (on(*min)?, on(*max)?);
                self.point(a)?;
                self.number(b.x - a.x)?;
                self.append(" ")?;
                self.number(b.y - a.y)?;
                self.append(" re W n\n")?;
            }
            Some(Clip::Circle { center, radius }) => {
                let circle = Seg2::Arc { center: *center, radius: *radius, start: 0.0, sweep: std::f64::consts::TAU };
                let start = on(*center + Vec2::new(*radius, 0.0))?;
                self.point(start)?;
                self.append("m\n")?;
                // Two half turns, each cut into cubics within the arc tolerance.
                for (from, sweep) in [(0.0, std::f64::consts::PI), (std::f64::consts::PI, std::f64::consts::PI)] {
                    let Seg2::Arc { center, radius, .. } = circle else { break };
                    let end = on(center + Vec2::from_angle(from + sweep) * radius)?;
                    let scaled = Seg2::Arc { center: center * PER_MM, radius: radius * PER_MM, start: from, sweep };
                    self.arc(scaled, end)?;
                }
                self.append("h W n\n")?;
            }
            None => {}
        }
        self.number(high.x - low.x)?;
        self.append(" 0 0 ")?;
        self.number(high.y - low.y)?;
        self.append(" ")?;
        self.point(low)?;
        self.append("cm\n/")?;
        self.append(name)?;
        self.append(" Do\nQ\n")
    }

    /// A path from its start; a closed one's closing line is left to the caller's `h` or `s`.
    fn path(&mut self, path: &GridPath) -> Result<()> {
        self.point(path.start)?;
        self.append("m\n")?;
        let mut at = path.start;
        for piece in &path.pieces {
            match *piece {
                Piece::Line { to } => {
                    self.point(to)?;
                    self.append("l\n")?;
                }
                Piece::Cubic { c1, c2, to } => {
                    self.point(c1)?;
                    self.point(c2)?;
                    self.point(to)?;
                    self.append("c\n")?;
                }
                // A parabola is the cubic with its control points two thirds of the way to `c`.
                Piece::Quadratic { c, to } => {
                    let (a, c, b) = (vec(at), vec(c), vec(to));
                    self.near(a + (c - a) * (2.0 / 3.0))?;
                    self.near(b + (c - b) * (2.0 / 3.0))?;
                    self.point(to)?;
                    self.append("c\n")?;
                }
                Piece::Arc { .. } => self.arc(seg(piece, at), piece.to())?,
            }
            at = piece.to();
        }
        Ok(())
    }

    /// An arc piece as cubics within [`ARC_TOLERANCE_MM`], ending exactly on `to`.
    fn arc(&mut self, arc: Seg2, to: GridPoint) -> Result<()> {
        let Seg2::Arc { center, radius, start, sweep } = arc else {
            return Err(IoError::Invalid("the sheet PDF: an arc piece isn't an arc".into()));
        };
        let n = cubics_for(radius, sweep).ok_or_else(|| IoError::Invalid("the sheet PDF: an arc needs too many cubics".into()))?;
        let h = sweep / n as f64;
        let k = 4.0 / 3.0 * (h / 4.0).tan() * radius;
        let tangent = |a: f64| Vec2::new(-a.sin(), a.cos());
        for i in 0..n {
            let (a0, a1) = (start + h * i as f64, start + h * (i + 1) as f64);
            let (p0, p3) = (center + Vec2::from_angle(a0) * radius, center + Vec2::from_angle(a1) * radius);
            self.near(p0 + tangent(a0) * k)?;
            self.near(p3 - tangent(a1) * k)?;
            if i + 1 == n {
                self.point(to)?
            } else {
                self.near(p3)?
            }
            self.append("c\n")?;
        }
        Ok(())
    }
}

/// How many equal cubics keep an arc of `radius` steps and `sweep` (at most a half turn, see
/// [`Piece::Arc`]) within the tolerance, each spanning at most a quarter turn. The 4/3·tan(θ/4)
/// cubic of a θ arc strays at most r·(2/27)·sin⁶(θ/4)/cos²(θ/4) from it, outward; each control
/// point rounded onto the grid moves the curve at most half a step's diagonal more.
fn cubics_for(radius: f64, sweep: f64) -> Option<usize> {
    let tolerance = ARC_TOLERANCE_MM * PER_MM - std::f64::consts::FRAC_1_SQRT_2;
    let strays = |theta: f64| radius * (2.0 / 27.0) * (theta / 4.0).sin().powi(6) / (theta / 4.0).cos().powi(2);
    // A quarter turn or less starts at one cubic, as it always has, so those arcs write the same bytes.
    let least = (sweep.abs() / FRAC_PI_2 - 1e-9).ceil().max(1.0);
    (sweep.is_finite() && sweep.abs() <= std::f64::consts::PI + 1e-9 && radius.is_finite())
        .then(|| (least as usize..=128).find(|&n| strays(sweep.abs() / n as f64) <= tolerance))
        .flatten()
}

/// Writes a planned picture as image XObject `image` (RGB) with its alpha, times its opacity, as
/// a soft mask (numbered from `refs`) where any pixel isn't opaque: its pixels asked for band by
/// band and deflated as they come. The bytes written, refused past `room`.
fn write_picture(pdf: &mut Pdf, image: Ref, refs: &mut Refs, planned: &PlannedPicture, pictures: &dyn Pictures, room: usize) -> Result<usize> {
    let source = planned.picture.source;
    let window = planned.window;
    let width = window.x1.saturating_sub(window.x0);
    let height = window.y1.saturating_sub(window.y0);
    let too_large = || IoError::Invalid(format!("picture {source}: the PDF's pictures would be over {} MiB", MAX_DRAWING_BYTES >> 20));
    let (mut rgb, mut mask) = (ZlibEncoder::new(Vec::new(), Compression::default()), ZlibEncoder::new(Vec::new(), Compression::default()));
    let opacity = planned.picture.opacity.clamp(0.0, 1.0);
    let mut opaque = true;
    let rows = (BAND_BYTES / (width as usize * 4).max(1)).max(1) as u32;
    let mut top = window.y0;
    let (mut row_rgb, mut row_alpha) = (Vec::new(), Vec::new());
    while top < window.y1 {
        let band = GridWindow { y0: top, y1: window.y1.min(top.saturating_add(rows)), ..window };
        let rgba = ask(pictures, source, &planned.grid, band)?;
        row_rgb.clear();
        row_alpha.clear();
        for &[r, g, b, a] in rgba.as_chunks::<4>().0 {
            row_rgb.extend_from_slice(&[r, g, b]);
            let a = (f64::from(a) * opacity).round() as u8;
            opaque &= a == u8::MAX;
            row_alpha.push(a);
        }
        let failed = |e: std::io::Error| IoError::Invalid(format!("picture {source}: {e}"));
        rgb.write_all(&row_rgb).map_err(failed)?;
        mask.write_all(&row_alpha).map_err(failed)?;
        if rgb.get_ref().len().saturating_add(mask.get_ref().len()) > room {
            return Err(too_large());
        }
        top = band.y1;
    }
    let failed = |e: std::io::Error| IoError::Invalid(format!("picture {source}: {e}"));
    let rgb = rgb.finish().map_err(failed)?;
    let mask = mask.finish().map_err(failed)?;
    let size = |n: u32| i32::try_from(n).map_err(|_| too_large());
    let (w, h) = (size(width)?, size(height)?);
    let written = rgb.len() + if opaque { 0 } else { mask.len() };
    if written > room {
        return Err(too_large());
    }
    let alpha = if opaque { None } else { Some(refs.next()?) };
    {
        let mut x = pdf.image_xobject(image, &rgb);
        x.filter(Filter::FlateDecode);
        x.width(w).height(h).color_space_name(Name(b"DeviceRGB")).bits_per_component(8);
        if let Some(alpha) = alpha {
            x.s_mask(alpha);
        }
    }
    if let Some(alpha) = alpha {
        let mut x = pdf.image_xobject(alpha, &mask);
        x.filter(Filter::FlateDecode);
        x.width(w).height(h).color_space_name(Name(b"DeviceGray")).bits_per_component(8);
    }
    Ok(written)
}

/// How many bytes of pixels are asked for at a time.
const BAND_BYTES: usize = 1 << 22;

// ---- fonts (P2) -------------------------------------------------------------------------------

/// The ToUnicode CMap's system: glyphs are named by CIDs of no registered collection.
const IDENTITY: SystemInfo<'static> = SystemInfo { registry: Str(b"Adobe"), ordering: Str(b"Identity"), supplement: 0 };

/// How an embedded font's glyphs are drawn: TrueType outlines (`glyf`), or CFF ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outlines {
    TrueType,
    Cff,
}

/// A font as the document's pages use it, gathered from every page's scene.
struct FontUse<'a> {
    font: &'a Font,
    /// Each glyph the runs place: its advance, in font units.
    advances: BTreeMap<u16, u16>,
    /// The text a glyph stands for, from the first cluster that starts with it.
    unicode: BTreeMap<u16, String>,
}

/// A font of the document: its resource name, and how it is carried.
struct PdfFont {
    id: [u8; 32],
    /// `F0`, `F1`… in order of id.
    tag: String,
    carried: Carried,
}

/// How a PDF carries a font ([`Embed`]).
enum Carried {
    /// As a Type 0 font whose CIDs are the embedded file's glyph ids.
    Embedded(Box<EmbeddedFont>),
    /// Not at all: its runs are filled with their glyphs' outlines, the text beside them.
    Outlines,
}

struct EmbeddedFont {
    type0: Ref,
    cid_font: Ref,
    descriptor: Ref,
    file: Ref,
    to_unicode: Ref,
    /// The scene's glyph id → the CID it is shown as.
    cids: BTreeMap<u16, u16>,
    /// Each CID's width in thousandths of an em, as written in `W` (so a run's pen follows it exactly).
    widths: BTreeMap<u16, f64>,
    /// The font file embedded, and its outlines.
    data: Vec<u8>,
    outlines: Outlines,
    /// The PostScript name, with a subset's tag.
    base_font: String,
}

/// The fonts every page's runs use, by id, each with the glyphs placed and their text.
fn font_uses<'a>(scenes: impl Iterator<Item = &'a Prepared<'a>>) -> Result<BTreeMap<[u8; 32], FontUse<'a>>> {
    let mut uses: BTreeMap<[u8; 32], FontUse<'a>> = BTreeMap::new();
    for scene in scenes {
        for group in scene.groups() {
            for item in &group.items {
                let Item::Text(run) = item else { continue };
                let font = scene.font(run).ok_or_else(|| IoError::Invalid("the sheet PDF: a run's font is not in its scene".into()))?;
                let entry = uses.entry(font.id).or_insert_with(|| FontUse { font, advances: BTreeMap::new(), unicode: BTreeMap::new() });
                for g in &run.glyphs {
                    // The font of a page holds the glyphs that page places, each with its advance.
                    let advance = font.glyphs.get(&g.id).map_or(0, |glyph| glyph.advance);
                    entry.advances.entry(g.id).or_insert(advance);
                }
                for cluster in &run.clusters {
                    let first = run.glyphs.get(cluster.glyphs.start as usize).map(|g| g.id);
                    let text = run.text.get(cluster.text.start as usize..cluster.text.end as usize);
                    if let (Some(first), Some(text)) = (first, text) {
                        entry.unicode.entry(first).or_insert_with(|| text.to_string());
                    }
                }
            }
        }
    }
    Ok(uses)
}

/// Each used font as the PDF carries it, its objects numbered from `refs`: embedded, as a subset
/// unless its licence says whole, or as outlines when it may not be embedded or can't be (a
/// collection's face that may not be subset, CFF outlines that may not be subset, a file the
/// subsetter refuses).
fn pdf_fonts(uses: &BTreeMap<[u8; 32], FontUse<'_>>, refs: &mut Refs) -> Result<Vec<PdfFont>> {
    let mut fonts = Vec::with_capacity(uses.len());
    for (i, (id, used)) in uses.iter().enumerate() {
        let carried = match embedded_file(used) {
            Some((data, outlines, cids, subset)) => {
                let font = used.font;
                let upm = f64::from(font.units_per_em);
                let widths =
                    cids.iter().filter_map(|(gid, cid)| used.advances.get(gid).map(|a| (*cid, thousandths(f64::from(*a) * 1000.0 / upm)))).collect();
                let base_font =
                    if subset { format!("{}+{}", subset_tag(id, cids.keys()), font.postscript_name) } else { font.postscript_name.clone() };
                Carried::Embedded(Box::new(EmbeddedFont {
                    type0: refs.next()?,
                    cid_font: refs.next()?,
                    descriptor: refs.next()?,
                    file: refs.next()?,
                    to_unicode: refs.next()?,
                    cids,
                    widths,
                    data,
                    outlines,
                    base_font,
                }))
            }
            None => Carried::Outlines,
        };
        fonts.push(PdfFont { id: *id, tag: format!("F{i}"), carried });
    }
    Ok(fonts)
}

/// The file a font is embedded as, the kind of its outlines, each placed glyph's CID in it, and
/// whether it is a subset; `None` when the font is drawn as outlines.
fn embedded_file(used: &FontUse<'_>) -> Option<(Vec<u8>, Outlines, BTreeMap<u16, u16>, bool)> {
    let font = used.font;
    match font.embed {
        Embed::Outlines => None,
        Embed::Subset => {
            // A face the subsetter can't read (or that trips it) is drawn as outlines, never a
            // failed export.
            let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut remapper = subsetter::GlyphRemapper::new();
                for gid in used.advances.keys() {
                    remapper.remap(*gid);
                }
                subsetter::subset(&font.bytes, font.face, &remapper).map(|data| (data, remapper))
            }));
            let (data, remapper) = made.ok()?.ok()?;
            let outlines = sfnt_outlines(&data)?;
            let cids = used.advances.keys().map(|gid| remapper.get(*gid).map(|cid| (*gid, cid))).collect::<Option<BTreeMap<_, _>>>()?;
            Some((data, outlines, cids, true))
        }
        // The whole face, as it is: only a single TrueType face can go in as its own file with its
        // glyph ids for CIDs (a collection's face would need cutting out; a CFF face's CIDs aren't
        // its glyph ids).
        Embed::Whole => {
            let single = font.face == 0 && font.bytes.get(0..4) != Some(b"ttcf");
            (single && sfnt_outlines(&font.bytes) == Some(Outlines::TrueType))
                .then(|| (font.bytes.to_vec(), Outlines::TrueType, used.advances.keys().map(|gid| (*gid, *gid)).collect(), false))
        }
    }
}

/// The outlines of an OpenType file: TrueType when it has `glyf`, CFF when it has `CFF `.
fn sfnt_outlines(data: &[u8]) -> Option<Outlines> {
    let tables = usize::from(u16::from_be_bytes([*data.get(4)?, *data.get(5)?]));
    (0..tables).find_map(|i| match data.get(12 + 16 * i..16 + 16 * i)? {
        b"glyf" => Some(Outlines::TrueType),
        b"CFF " => Some(Outlines::Cff),
        _ => None,
    })
}

/// A subset's six-letter tag (ISO 32000-1 9.6.4), from its font and glyphs, so the same subset
/// always has the same name.
fn subset_tag<'a>(id: &[u8; 32], glyphs: impl Iterator<Item = &'a u16>) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in id.iter().copied().chain(glyphs.flat_map(|g| g.to_be_bytes())) {
        h = (h ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    (0..6).map(|k| char::from(b'A' + ((h >> (k * 5)) % 26) as u8)).collect()
}

/// `x` rounded to thousandths, as a width is written.
fn thousandths(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// A real number as PDF writes it: at most `decimals` decimals, no exponent, no "-0".
fn real(x: f64, decimals: usize) -> String {
    let s = format!("{x:.decimals$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if s == "-0" { "0".to_string() } else { s }
}

/// Writes the document's embedded fonts: each Type 0 font, its CID font, descriptor, file and
/// ToUnicode CMap.
fn write_fonts(pdf: &mut Pdf, fonts: &[PdfFont], uses: &BTreeMap<[u8; 32], FontUse<'_>>) -> Result<()> {
    for font in fonts {
        let Carried::Embedded(e) = &font.carried else { continue };
        let Some(used) = uses.get(&font.id) else { continue };
        let source = used.font;
        let upm = f64::from(source.units_per_em);
        let em = |v: i16| (f64::from(v) * 1000.0 / upm) as f32;
        let name = Name(e.base_font.as_bytes());
        pdf.type0_font(e.type0).base_font(name).encoding_predefined(Name(b"Identity-H")).descendant_font(e.cid_font).to_unicode(e.to_unicode);
        {
            let mut cid = pdf.cid_font(e.cid_font);
            cid.subtype(if e.outlines == Outlines::TrueType { CidFontType::Type2 } else { CidFontType::Type0 });
            cid.base_font(name).system_info(IDENTITY).font_descriptor(e.descriptor).default_width(0.0);
            {
                let mut widths = cid.widths();
                for (c, w) in &e.widths {
                    widths.consecutive(*c, [*w as f32]);
                }
            }
            if e.outlines == Outlines::TrueType {
                cid.cid_to_gid_map_predefined(Name(b"Identity"));
            }
        }
        {
            let m = source.metrics;
            let mut flags = PdfFontFlags::empty();
            flags.set(PdfFontFlags::FIXED_PITCH, m.flags.fixed_pitch);
            flags.set(PdfFontFlags::SERIF, m.flags.serif);
            flags.set(PdfFontFlags::ITALIC, m.flags.italic);
            flags.set(if m.flags.symbolic { PdfFontFlags::SYMBOLIC } else { PdfFontFlags::NON_SYMBOLIC }, true);
            let mut d = pdf.font_descriptor(e.descriptor);
            d.name(name).flags(flags);
            d.bbox(Rect::new(em(m.bbox[0]), em(m.bbox[1]), em(m.bbox[2]), em(m.bbox[3])));
            d.italic_angle(m.italic_angle as f32).ascent(em(m.ascent)).descent(em(m.descent)).cap_height(em(m.cap_height));
            d.stem_v((f64::from(m.stem_v) * 1000.0 / upm) as f32);
            if e.outlines == Outlines::TrueType {
                d.font_file2(e.file);
            } else {
                d.font_file3(e.file);
            }
        }
        let compressed = deflate(&e.data)?;
        {
            let mut file = pdf.stream(e.file, &compressed);
            file.filter(Filter::FlateDecode);
            if e.outlines == Outlines::TrueType {
                file.pair(Name(b"Length1"), i32::try_from(e.data.len()).map_err(|_| IoError::Invalid("a font over 2 GiB".into()))?);
            } else {
                file.pair(Name(b"Subtype"), Name(b"OpenType"));
            }
        }
        let mut cmap = UnicodeCmap::new(Name(b"SolidCraft-UCS"), IDENTITY);
        for (gid, text) in &used.unicode {
            if let Some(c) = e.cids.get(gid).filter(|_| !text.is_empty()) {
                cmap.pair_with_multiple(*c, text.chars());
            }
        }
        let cmap = cmap.finish();
        pdf.cmap(e.to_unicode, &cmap);
    }
    Ok(())
}

/// `data`, deflated.
fn deflate(data: &[u8]) -> Result<Vec<u8>> {
    let mut deflate = ZlibEncoder::new(Vec::new(), Compression::default());
    deflate.write_all(data).and_then(|()| deflate.finish()).map_err(|e| IoError::Invalid(format!("the sheet PDF: {e}")))
}

/// `text` as a PDF text string in hex: UTF-16BE with its byte order mark.
fn utf16_hex(text: &str) -> String {
    let mut out = String::with_capacity(4 + text.len() * 4);
    out.push_str("FEFF");
    for unit in text.encode_utf16() {
        out.push_str(&format!("{unit:04X}"));
    }
    out
}

#[cfg(test)]
#[path = "pdf_tests.rs"]
mod tests;
