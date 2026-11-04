//! The internet, for the assistant (docs/agent-chat.md, "The internet"): `web.fetch` reads a
//! page (its title, text and links) and `file.download` saves the CAD files a link gives, a
//! vendor's zip unpacked, in a folder of the user's. Both go through [`crate::net`], which only
//! fetches public https:// addresses and only while the user allows it. Neither changes the
//! design: the assistant inserts what it downloaded with the commands that insert files.

use serde_json::Value;

use super::CommandSpec;
use crate::params::bad;
use crate::schema::web::{DownloadedFile, FileDownloadParams, FileDownloadReply, WebFetchParams, WebFetchReply, WebLink};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session};

const FETCH_CONTRACT: TypedContract<WebFetchParams> = TypedContract::new();
const DOWNLOAD_CONTRACT: TypedContract<FileDownloadParams> = TypedContract::new();

/// The most of a page read: a larger page is refused.
const MAX_PAGE_BYTES: usize = 8 << 20;
/// Text returned when the call names no limit.
const DEFAULT_CHARS: usize = 20_000;
/// Links listed at most.
const MAX_LINKS: usize = 300;
/// CAD files one zip may give.
const MAX_ZIP_FILES: usize = 50;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("web.fetch", "Read Web Page", crate::checked!("web.fetch", fetch))
        .icon("chain")
        .noundo()
        .contract(&FETCH_CONTRACT)
        .output(output_schema_for::<WebFetchReply>)
        .params(
            "url: an https:// address (never one on this computer's network); max_chars?: most characters of text (default 20000, \
             1000 to 100000) → url (after redirects), status, title?, text (the page without its markup), truncated, links: \
             [{text, url}] (at most 300), cad_links: the links naming a CAD file or a zip (what file.download takes). Needs the \
             internet allowed (Preferences → AI); never changes the design",
        ),
    CommandSpec::new("file.download", "Download CAD File", crate::checked!("file.download", download))
        .icon("import")
        .noundo()
        .contract(&DOWNLOAD_CONTRACT)
        .output(output_schema_for::<FileDownloadReply>)
        .params(
            "url: an https:// address of a STEP, IGES or mesh file, or a zip holding them (a page that asks to sign in is refused: \
             read it with web.fetch for the file's own link); folder?: where to save (a full path; default the first Parts \
             Library's Downloads folder, else Downloads/SolidCraft); name?: the name for a single file (default the site's) → from, \
             folder, files: [{file, format, bytes}] (a zip's CAD files each; a name in use gets ' (2)'). Never changes the design: \
             insert a STEP with library.parts.convert then component.insert_design, or file.insert_step; a mesh with \
             file.insert_mesh",
        ),
];

fn fetch(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "web.fetch";
    let url = p.get("url").and_then(Value::as_str).unwrap_or_default();
    let max_chars = p.get("max_chars").and_then(Value::as_u64).map_or(DEFAULT_CHARS, |n| n as usize);
    let got = crate::net::get(url, MAX_PAGE_BYTES).map_err(|e| bad(cmd, e))?;
    let body = String::from_utf8_lossy(&got.body);
    let html = got.content_type.contains("html") || (got.content_type.is_empty() && body.trim_start().starts_with('<'));
    let readable = html || got.content_type.starts_with("text/") || got.content_type.contains("json") || got.content_type.contains("xml");
    if !readable {
        return Err(bad(cmd, format!("{} is not a web page ({}): for a CAD file use file.download", got.url, got.content_type)));
    }
    let (title, text, links) = if html { page(&body, &got.url) } else { (None, tidy(&body), Vec::new()) };
    let truncated = text.chars().count() > max_chars;
    let text: String = text.chars().take(max_chars).collect();
    let cad_links = links.iter().filter(|l| names_cad_file(&l.url)).map(|l| l.url.clone()).collect();
    let reply = WebFetchReply { url: got.url, status: got.status, title, text, truncated, links, cad_links };
    serde_json::to_value(reply).map_err(|e| bad(cmd, e.to_string()))
}

fn download(_s: &mut Session, p: &Value) -> Result<Value> {
    use solidcraft_io::download::{cad_files_in_zip, is_cad_path, is_zip_name, safe_name, sniffed_extension};
    let cmd = "file.download";
    let url = github_raw(p.get("url").and_then(Value::as_str).unwrap_or_default());
    let limit = solidcraft_io::limits::max_step_bytes();
    let got = crate::net::get(&url, limit.saturating_add(1)).map_err(|e| bad(cmd, e))?;
    if !(200..300).contains(&got.status) {
        return Err(bad(cmd, format!("{} answered {} instead of the file", got.url, got.status)));
    }
    if got.body.len() > limit {
        return Err(bad(cmd, format!("{} is larger than SolidCraft reads", got.url)));
    }
    let sniffed = sniffed_extension(&got.body);
    if got.content_type.contains("html") && sniffed.is_none() {
        return Err(bad(
            cmd,
            format!(
                "{} is a web page, not a CAD file (a site that asks you to sign in sends one): read it with web.fetch for the file's own link",
                got.url
            ),
        ));
    }
    // The name: the one asked for, the site's, the address's last part, else what the bytes are.
    let from_url = last_segment(&got.url);
    let given = p.get("name").and_then(Value::as_str);
    let mut name = given.and_then(safe_name).or_else(|| got.file_name.as_deref().and_then(safe_name)).or_else(|| safe_name(&from_url));
    if let (None, Some(ext)) = (&name, sniffed) {
        let stem: String = from_url
            .rsplit_once('.')
            .map_or(from_url.as_str(), |(s, _)| s)
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' '))
            .take(80)
            .collect();
        name = safe_name(&format!("{}.{ext}", if stem.trim().is_empty() { "download" } else { stem.trim() }));
    }
    let Some(name) = name else {
        return Err(bad(cmd, format!("{} is not a CAD file SolidCraft reads (STEP, IGES, STL, 3MF, OBJ, glTF, or a zip of them)", got.url)));
    };
    let files: Vec<(String, Vec<u8>)> = if is_zip_name(&name) || sniffed == Some("zip") {
        let inside = cad_files_in_zip(&got.body, MAX_ZIP_FILES, limit).map_err(|e| bad(cmd, e))?;
        if inside.is_empty() {
            return Err(bad(cmd, format!("the zip file from {} holds no CAD file SolidCraft reads", got.url)));
        }
        inside
    } else if is_cad_path(&name) {
        vec![(name, got.body)]
    } else {
        return Err(bad(cmd, format!("{name} is not a CAD file SolidCraft reads")));
    };
    let folder = match p.get("folder").and_then(Value::as_str).filter(|f| !f.trim().is_empty()) {
        Some(f) if std::path::Path::new(f).is_absolute() => std::path::PathBuf::from(f),
        Some(f) => return Err(bad(cmd, format!("`folder` must be a full path, not {f}"))),
        None => crate::net::downloads_dir().ok_or_else(|| bad(cmd, "this computer has no Downloads folder: give `folder`"))?,
    };
    std::fs::create_dir_all(&folder).map_err(|e| bad(cmd, format!("{}: {e}", folder.display())))?;
    let mut saved = Vec::with_capacity(files.len());
    for (name, bytes) in files {
        let path = unused(&folder, &name);
        solidcraft_io::write_atomic(&path, &bytes, false).map_err(|e| bad(cmd, format!("{}: {e}", path.display())))?;
        let format = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        saved.push(DownloadedFile { file: path.to_string_lossy().into_owned(), format, bytes: bytes.len() as u64 });
    }
    let reply = FileDownloadReply { from: got.url, folder: folder.to_string_lossy().into_owned(), files: saved };
    serde_json::to_value(reply).map_err(|e| bad(cmd, e.to_string()))
}

/// A GitHub file page's address (`github.com/owner/repo/blob/branch/path`) as the file's own
/// (`raw.githubusercontent.com/owner/repo/branch/path`): the page is HTML about the file, and
/// makers often keep their CAD files on GitHub. Any other address as it is.
fn github_raw(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    for prefix in ["https://github.com/", "https://www.github.com/"] {
        if lower.starts_with(prefix)
            && let Some(rest) = url.get(prefix.len()..)
        {
            let parts: Vec<&str> = rest.splitn(4, '/').collect();
            if let [owner, repo, "blob", path] = parts.as_slice() {
                let path = path.split(['?', '#']).next().unwrap_or_default();
                return format!("https://raw.githubusercontent.com/{owner}/{repo}/{path}");
            }
        }
    }
    url.to_string()
}

/// `name` in `folder`, or `name (2)`, `name (3)`, … when it is taken: a download never replaces a
/// file.
fn unused(folder: &std::path::Path, name: &str) -> std::path::PathBuf {
    let (stem, ext) = name.rsplit_once('.').map_or((name, None), |(s, e)| (s, Some(e)));
    (1..10_000)
        .map(|n| {
            let file = match (n, ext) {
                (1, _) => name.to_string(),
                (_, Some(e)) => format!("{stem} ({n}).{e}"),
                (_, None) => format!("{stem} ({n})"),
            };
            folder.join(file)
        })
        .find(|p| !p.exists())
        .unwrap_or_else(|| folder.join(name))
}

/// The last part of an address's path, percent-decoded.
fn last_segment(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let last = path.rsplit('/').next().unwrap_or_default();
    percent_decoded(last)
}

fn percent_decoded(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match (b[i], b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (c, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Does `url` name a CAD file or a zip (by its path's extension)?
fn names_cad_file(url: &str) -> bool {
    let last = last_segment(url);
    solidcraft_io::download::is_cad_path(&last) || solidcraft_io::download::is_zip_name(&last)
}

/// A page's title, its text without markup, and its links resolved against `base`.
fn page(html: &str, base: &str) -> (Option<String>, String, Vec<WebLink>) {
    let lower = html.to_ascii_lowercase();
    let title = element_text(html, &lower, "title").map(|t| tidy(&decode(&t))).filter(|t| !t.is_empty());
    // What a reader never sees.
    let mut hidden: Vec<(usize, usize)> = Vec::new();
    for tag in ["script", "style", "noscript", "svg", "template", "head"] {
        let (open, close) = (format!("<{tag}"), format!("</{tag}"));
        let mut at = 0;
        while let Some(s) = lower.get(at..).and_then(|r| r.find(&open)).map(|i| i + at) {
            // `<head` is not `<header`.
            if !lower.as_bytes().get(s + open.len()).is_some_and(|c| c.is_ascii_whitespace() || matches!(c, b'>' | b'/')) {
                at = s + open.len();
                continue;
            }
            let end = lower.get(s..).and_then(|r| r.find(&close)).map_or(lower.len(), |i| s + i);
            let end = lower.get(end..).and_then(|r| r.find('>')).map_or(lower.len(), |i| end + i + 1);
            hidden.push((s, end));
            at = end;
        }
    }
    hidden.sort_unstable();
    let is_hidden = |i: usize| hidden.iter().any(|(a, b)| (*a..*b).contains(&i));
    let mut text = String::new();
    let mut links: Vec<WebLink> = Vec::new();
    let mut open_link: Option<(String, usize)> = None;
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if is_hidden(i) {
            i = hidden.iter().find(|(a, b)| (*a..*b).contains(&i)).map_or(i + 1, |(_, b)| *b);
            continue;
        }
        if bytes[i] == b'<' {
            let end = html.get(i..).and_then(|r| r.find('>')).map_or(bytes.len(), |e| i + e + 1);
            let tag = lower.get(i + 1..end.saturating_sub(1)).unwrap_or_default();
            let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if name == "a" && !tag.starts_with('/') {
                let raw = html.get(i + 1..end.saturating_sub(1)).unwrap_or_default();
                open_link = attribute(raw, "href").and_then(|h| resolve(base, &decode(&h))).map(|u| (u, text.len()));
            } else if name == "a"
                && let Some((url, from)) = open_link.take()
            {
                let label = tidy(text.get(from..).unwrap_or_default());
                if links.len() < MAX_LINKS && !links.iter().any(|l| l.url == url) {
                    links.push(WebLink { text: label.chars().take(200).collect(), url });
                }
            }
            let block = [
                "p", "div", "br", "li", "tr", "td", "th", "h1", "h2", "h3", "h4", "h5", "h6", "table", "ul", "ol", "section", "article", "header",
                "footer", "nav", "main", "form", "dt", "dd",
            ];
            // A block starts a line; an inline element (a link, bold text) adds nothing, as in a browser.
            if block.contains(&name.as_str()) {
                text.push('\n');
            }
            i = end;
            continue;
        }
        let next = html.get(i..).and_then(|r| r.find('<')).map_or(bytes.len(), |n| i + n);
        let next = hidden.iter().filter(|(a, _)| *a > i).map(|(a, _)| *a).min().map_or(next, |h| h.min(next));
        text.push_str(&decode(html.get(i..next).unwrap_or_default()));
        i = next.max(i + 1);
    }
    (title, tidy(&text), links)
}

/// The text inside the first `<tag>` element.
fn element_text(html: &str, lower: &str, tag: &str) -> Option<String> {
    let s = lower.find(&format!("<{tag}"))?;
    let open_end = s + lower.get(s..)?.find('>')? + 1;
    let close = open_end + lower.get(open_end..)?.find(&format!("</{tag}"))?;
    html.get(open_end..close).map(str::to_string)
}

/// The value of attribute `name` in a tag's inside (`a href="…" class=x`).
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower.get(from..).and_then(|r| r.find(name)).map(|i| i + from) {
        let before_ok = at == 0 || lower.as_bytes().get(at - 1).is_some_and(|c| c.is_ascii_whitespace());
        let rest = tag.get(at + name.len()..)?.trim_start();
        if before_ok && let Some(value) = rest.strip_prefix('=') {
            let value = value.trim_start();
            return match value.chars().next()? {
                q @ ('"' | '\'') => value.get(1..)?.split(q).next().map(str::to_string),
                _ => value.split(|c: char| c.is_whitespace() || c == '>').next().map(str::to_string),
            };
        }
        from = at + name.len();
    }
    None
}

/// `href` as a full address from the page at `base`; `None` for a script, an e-mail, a phone
/// number or a link within the page.
fn resolve(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    let lower = href.to_ascii_lowercase();
    if href.is_empty() || href.starts_with('#') || ["javascript:", "mailto:", "tel:", "data:"].iter().any(|s| lower.starts_with(s)) {
        return None;
    }
    let href = href.split('#').next().unwrap_or_default();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return Some(href.to_string());
    }
    let (scheme, rest) = base.split_once("://")?;
    if let Some(r) = href.strip_prefix("//") {
        return Some(format!("{scheme}://{r}"));
    }
    let host_end = rest.find('/').unwrap_or(rest.len());
    let origin = format!("{scheme}://{}", rest.get(..host_end)?);
    let path = rest.get(host_end..).unwrap_or("/").split(['?', '#']).next().unwrap_or("/");
    if href.starts_with('/') {
        return Some(format!("{origin}{href}"));
    }
    if href.starts_with('?') {
        return Some(format!("{origin}{path}{href}"));
    }
    let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    let (rel, query) = href.split_once('?').map_or((href, None), |(r, q)| (r, Some(q)));
    for seg in rel.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let joined = format!("{origin}/{}", parts.join("/"));
    Some(match query {
        Some(q) => format!("{joined}?{q}"),
        None => joined,
    })
}

/// HTML's character references in `s` (the named ones a page commonly has, and numeric ones).
fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let semi = tail.char_indices().take(12).find(|(_, c)| *c == ';').map(|(i, _)| i);
        let named = semi.map(|e| &tail[1..e]);
        let ch = named.and_then(|n| match n {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "deg" => Some('°'),
            "plusmn" => Some('±'),
            n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        });
        match (ch, semi) {
            (Some(c), Some(e)) => {
                out.push(c);
                rest = &tail[e + 1..];
            }
            _ => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `s` with runs of spaces made one, lines trimmed and at most one blank line in a row.
fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0;
    for line in s.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank += 1;
            if blank == 1 && !out.is_empty() {
                out.push('\n');
            }
            continue;
        }
        blank = 0;
        out.push_str(&line);
        out.push('\n');
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn a_page_gives_its_title_text_and_resolved_links() {
        let html = r##"<html><head><title>T200 Thruster &amp; CAD</title><style>p{color:red}</style><script>var x="<a href='/no'>";</script></head>
            <body><header>Blue Robotics</header><h1>T200</h1><p>Rated 5&nbsp;kgf &mdash; see <a href="/files/T200.STEP">the STEP file</a> or <a href='cad/T200.zip'>a zip</a>.</p>
            <a href="#top">top</a><a href="mailto:x@y.z">mail</a><a href="https://other.example/x">other</a><a href="/files/T200.STEP">again</a></body></html>"##;
        let (title, text, links) = page(html, "https://bluerobotics.com/store/thrusters/t200/");
        assert_eq!(title.as_deref(), Some("T200 Thruster & CAD"));
        assert!(text.contains("Rated 5 kgf — see the STEP file or a zip."), "{text}");
        assert!(!text.contains("color") && !text.contains("var x"), "{text}");
        assert!(text.starts_with("Blue Robotics"), "a header is not the head: {text}");
        let urls: Vec<&str> = links.iter().map(|l| l.url.as_str()).collect();
        assert_eq!(
            urls,
            ["https://bluerobotics.com/files/T200.STEP", "https://bluerobotics.com/store/thrusters/t200/cad/T200.zip", "https://other.example/x"]
        );
        assert_eq!(links[0].text, "the STEP file");
        assert!(names_cad_file(&links[0].url) && names_cad_file(&links[1].url) && !names_cad_file(&links[2].url));
    }

    #[test]
    fn links_resolve_as_a_browser_would() {
        let base = "https://example.com/a/b/page.html?x=1";
        assert_eq!(resolve(base, "c.step").as_deref(), Some("https://example.com/a/b/c.step"));
        assert_eq!(resolve(base, "../c.step").as_deref(), Some("https://example.com/a/c.step"));
        assert_eq!(resolve(base, "/c.step?dl=1#f").as_deref(), Some("https://example.com/c.step?dl=1"));
        assert_eq!(resolve(base, "//cdn.example.com/c.step").as_deref(), Some("https://cdn.example.com/c.step"));
        assert_eq!(resolve(base, "?page=2").as_deref(), Some("https://example.com/a/b/page.html?page=2"));
        for none in ["#top", "javascript:void(0)", "mailto:a@b", ""] {
            assert_eq!(resolve(base, none), None, "{none}");
        }
        assert_eq!(last_segment("https://x.com/dl/My%20Part.STEP?token=1"), "My Part.STEP");
        assert_eq!(decode("&lt;b&gt; &#176; &#x2013; &bogus; &"), "<b> ° – &bogus; &");
    }

    #[test]
    fn a_github_file_page_downloads_the_file_itself() {
        assert_eq!(
            github_raw("https://github.com/bluerobotics/T200/blob/main/CAD/T200.STEP?raw=true"),
            "https://raw.githubusercontent.com/bluerobotics/T200/main/CAD/T200.STEP"
        );
        for same in ["https://github.com/bluerobotics/T200", "https://example.com/a/blob/b/c.step", "https://github.com/a/b/tree/main/cad"] {
            assert_eq!(github_raw(same), same);
        }
    }

    #[test]
    fn a_download_is_saved_beside_others_never_over_them() {
        let dir = std::env::temp_dir().join(format!("solidcraft-download-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("T200.step"), b"old").unwrap();
        assert_eq!(unused(&dir, "T200.step"), dir.join("T200 (2).step"));
        assert_eq!(unused(&dir, "New.stl"), dir.join("New.stl"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
