//! Send report: a fork's test builds send the reviewed report to their developer's report service
//! (`tools/report-worker`, docs/fork-releases.md), which files it as an issue on the fork. Only
//! what the form shows goes: the title, the answers, the technical details and the screenshot,
//! never the design. The host does the HTTP (`Services::send_report`); without it (every
//! upstream build, the browser) the form only saves and copies, as before.

use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use serde_json::{Value, json};

use super::Draft;
use crate::SolveApp;
use crate::i18n::{format, tr};

/// What the host answers: the service's HTTP status and body, or why there was none.
pub type Answer = Result<(u16, Vec<u8>), String>;

/// POSTs a report (JSON bytes) to this build's report service and answers on the channel.
pub type SendReport = Arc<dyn Fn(Vec<u8>, Sender<Answer>) + Send + Sync>;

/// A screenshot bigger than this as PNG is left out, so a report stays well under what the
/// service accepts.
const MAX_SCREENSHOT: usize = 6 * 1024 * 1024;
/// The most of an error message from the service that is shown.
const MAX_ERROR: usize = 300;

/// The send in flight (one at a time).
#[derive(Default)]
pub struct Sending {
    rx: std::sync::Mutex<Option<Receiver<Answer>>>,
}

impl Sending {
    fn busy(&self) -> bool {
        self.rx.lock().is_ok_and(|rx| rx.is_some())
    }
}

/// Whether this build sends reports.
pub fn available(app: &SolveApp) -> bool {
    app.services.send_report.is_some()
}

/// What a report is, for "already sent": the title, the text and whether a screenshot goes.
pub(super) fn fingerprint(draft: &Draft) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    draft.report_title().hash(&mut h);
    draft.report_body().hash(&mut h);
    draft.image.as_ref().map(|i| i.size).hash(&mut h);
    h.finish()
}

/// The request body: plain JSON the service checks again before filing anything.
pub(super) fn payload(title: &str, body: &str, version: &str, os: &str, screenshot_png: Option<&[u8]>) -> Vec<u8> {
    let mut v = json!({ "title": title, "body": body, "version": version, "os": os });
    if let Some(png) = screenshot_png.filter(|p| p.len() <= MAX_SCREENSHOT) {
        v["screenshot_png"] = json!(solidcraft_engine::doc::canvas::base64_encode(png));
    }
    serde_json::to_vec(&v).unwrap_or_default()
}

/// The issue's address from the service's answer, or why the report was not filed.
pub(super) fn parse_answer(answer: Answer) -> Result<String, String> {
    let (status, body) = answer?;
    let json: Option<Value> = serde_json::from_slice(&body).ok();
    let field = |k: &str| json.as_ref().and_then(|j| j.get(k)).and_then(Value::as_str).map(|s| super::limited(s, MAX_ERROR));
    if !(200..300).contains(&status) {
        return Err(field("error").unwrap_or_else(|| format!("the report service answered HTTP {status}")));
    }
    match field("url") {
        Some(url) if url.starts_with("https://github.com/") && !url.contains(char::is_whitespace) => Ok(url),
        _ => Err("the report service gave no issue address".into()),
    }
}

/// Send the reviewed draft, unless a send is running or this very report went already.
pub(super) fn start(app: &mut SolveApp, draft: &mut Draft) {
    let Some(send) = app.services.send_report.clone() else { return };
    let print = fingerprint(draft);
    if app.issue_report.sending.busy() || draft.sent == Some(print) {
        return;
    }
    let screenshot = match draft.image.as_deref().map(super::png) {
        Some(Ok(png)) => Some(png),
        Some(Err(e)) => {
            log::warn!("bug report screenshot left out: {e}");
            None
        }
        None => None,
    };
    let os = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
    let body = payload(&draft.report_title(), &draft.report_body(), crate::update::current_version(), &os, screenshot.as_deref());
    let (tx, rx) = channel();
    if let Ok(mut slot) = app.issue_report.sending.rx.lock() {
        *slot = Some(rx);
    }
    draft.sending = Some(print);
    draft.note = tr("ui.bug_report_sending").into();
    send(body, tx);
}

/// Per frame: the answer of a send, into the draft (or the status bar when it was closed).
pub(super) fn poll(app: &mut SolveApp, ctx: &egui::Context) {
    let answer = {
        let Ok(mut slot) = app.issue_report.sending.rx.lock() else { return };
        let Some(rx) = slot.as_ref() else { return };
        match rx.try_recv() {
            Ok(answer) => {
                *slot = None;
                answer
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                *slot = None;
                Err("the upload stopped".into())
            }
        }
    };
    let _language = crate::i18n::scope(&app.preferences.language);
    let (message, error) = match parse_answer(answer) {
        Ok(url) => (format("ui.bug_report_sent", &[("url", &url)]), false),
        Err(e) => (format("ui.bug_report_send_failed", &[("error", &e)]), true),
    };
    match app.issue_report.draft.as_mut() {
        Some(draft) => {
            if !error {
                draft.sent = draft.sending;
            }
            draft.sending = None;
            draft.note = message;
        }
        None => app.set_status(message, error),
    }
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn the_payload_holds_what_the_form_shows_and_no_more() {
        let p: Value =
            serde_json::from_slice(&payload("Fillet: fails", "## Actual behavior\nx", "0.24.0-test.3", "macos aarch64", Some(b"png"))).unwrap();
        assert_eq!(p["title"], "Fillet: fails");
        assert_eq!(p["version"], "0.24.0-test.3");
        assert_eq!(p["screenshot_png"], "cG5n");
        assert_eq!(p.as_object().unwrap().len(), 5, "title, body, version, os, screenshot: {p}");
        // A huge screenshot is left out rather than refused.
        let big = vec![0u8; MAX_SCREENSHOT + 1];
        let p: Value = serde_json::from_slice(&payload("t", "b", "v", "o", Some(&big))).unwrap();
        assert!(p.get("screenshot_png").is_none());
    }

    #[test]
    fn only_a_github_issue_address_counts_as_sent() {
        let ok = |status: u16, body: &str| parse_answer(Ok((status, body.as_bytes().to_vec())));
        assert_eq!(ok(201, r#"{"url":"https://github.com/kaztre/solidcraft/issues/12"}"#).unwrap(), "https://github.com/kaztre/solidcraft/issues/12");
        assert!(ok(201, r#"{"url":"https://evil.example/x"}"#).is_err());
        assert!(ok(201, r#"{"url":"https://github.com/x y"}"#).is_err());
        assert!(ok(201, "not json").is_err());
        assert_eq!(ok(429, r#"{"error":"too many reports, try again later"}"#).unwrap_err(), "too many reports, try again later");
        assert_eq!(ok(500, "").unwrap_err(), "the report service answered HTTP 500");
        assert_eq!(parse_answer(Err("offline".into())).unwrap_err(), "offline");
        // A hostile error message is bounded.
        let long = format!(r#"{{"error":"{}"}}"#, "x".repeat(10_000));
        assert!(ok(400, &long).unwrap_err().len() <= MAX_ERROR + 8);
    }
}
