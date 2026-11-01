//! Open on GitHub: a fork's test builds (docs/fork-releases.md) open the reviewed report as a new
//! issue on the fork in the tester's browser, where they sign in and submit it. The app uploads
//! nothing: the title and the text travel in the address, and the tester sees them on GitHub
//! before anything is filed. Without a fork (every upstream build) the form only saves and copies.

use super::Draft;
use crate::SolveApp;
use crate::i18n::{format, tr};

/// The longest address that browsers and GitHub reliably take; a longer report is cut short in
/// the address and copied whole to the clipboard.
const MAX_URL: usize = 8000;

/// The fork this build files reports on, if any.
pub(super) fn repo(app: &SolveApp) -> Option<&'static str> {
    app.issue_report.github
}

/// `text` percent-encoded for a query value: everything but the unreserved characters.
fn encode(text: &str, out: &mut String) {
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&std::format!("%{b:02X}"));
        }
    }
}

/// The new-issue address for this report, and whether the body had to be cut to fit.
pub(super) fn issue_url(repo: &str, title: &str, body: &str) -> (String, bool) {
    let mut url = std::format!("https://github.com/{repo}/issues/new?labels=bug&title=");
    encode(title, &mut url);
    url.push_str("&body=");
    let mut whole = String::new();
    encode(body, &mut whole);
    if url.len() + whole.len() <= MAX_URL {
        url.push_str(&whole);
        return (url, false);
    }
    let mut marker = String::new();
    encode("\n\n[Cut short to fit the address. Paste the full report from the clipboard here.]\n", &mut marker);
    let room = MAX_URL.saturating_sub(url.len() + marker.len());
    let mut kept = String::new();
    let mut piece = String::new();
    for c in body.chars() {
        piece.clear();
        encode(c.encode_utf8(&mut [0; 4]), &mut piece);
        if kept.len() + piece.len() > room {
            break;
        }
        kept.push_str(&piece);
    }
    url.push_str(&kept);
    url.push_str(&marker);
    (url, true)
}

/// Open the reviewed draft as a new issue on the fork (a saved copy, when there is one, holds the
/// screenshot to attach there).
pub(super) fn open(app: &SolveApp, draft: &mut Draft, ctx: &egui::Context) {
    let Some(repo) = repo(app) else { return };
    let body = draft.report_body();
    let (url, cut) = issue_url(repo, &draft.report_title(), &body);
    ctx.open_url(egui::OpenUrl::new_tab(url));
    let mut note = tr("ui.bug_report_github_opened").to_string();
    let screenshot = app
        .issue_report
        .dir
        .as_deref()
        .zip(draft.saved.as_deref())
        .and_then(|(dir, id)| solidcraft_engine::bug_report::read(dir, id).ok())
        .and_then(|r| r["screenshot_path"].as_str().map(str::to_string));
    if let Some(path) = screenshot {
        note.push(' ');
        note.push_str(&format("ui.bug_report_github_screenshot", &[("path", &path)]));
    }
    if cut {
        ctx.copy_text(body);
        note.push(' ');
        note.push_str(tr("ui.bug_report_github_cut"));
    }
    draft.note = note;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn a_report_opens_as_a_new_issue_with_its_title_and_text_encoded() {
        let (url, cut) = issue_url("kaztre/solidcraft", "Fillet: fails & stops", "## Actual behavior\nIt failed 100%");
        assert!(!cut);
        assert_eq!(
            url,
            "https://github.com/kaztre/solidcraft/issues/new?labels=bug&title=Fillet%3A%20fails%20%26%20stops&body=%23%23%20Actual%20behavior%0AIt%20failed%20100%25"
        );
    }

    #[test]
    fn a_long_report_is_cut_short_within_the_address_limit_on_a_character() {
        for body in ["d".repeat(40_000), "調査🚀".repeat(10_000)] {
            let (url, cut) = issue_url("kaztre/solidcraft", "t", &body);
            assert!(cut && url.len() <= MAX_URL, "{}", url.len());
            assert!(url.ends_with("%5D%0A"), "the marker closes the body");
            // The cut falls between characters, so the browser decodes the body to valid text.
            let query = url.split_once("&body=").unwrap().1.as_bytes();
            let (mut bytes, mut i) = (Vec::new(), 0);
            while i < query.len() {
                if query[i] == b'%' {
                    bytes.push(u8::from_str_radix(std::str::from_utf8(&query[i + 1..i + 3]).unwrap(), 16).unwrap());
                    i += 3;
                } else {
                    bytes.push(query[i]);
                    i += 1;
                }
            }
            let text = String::from_utf8(bytes).unwrap();
            assert!(body.starts_with(text.split("\n\n[Cut short").next().unwrap()));
        }
    }
}
