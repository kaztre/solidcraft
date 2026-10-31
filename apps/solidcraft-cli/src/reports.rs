//! `solidcraft-cli reports`: the bug reports written with Report a bug on this computer
//! (`solidcraft_engine::bug_report`, docs/crash-reports.md): list them, show one, replay one,
//! mark one fixed, write one as an issue, delete them.

use serde_json::{Value, json};
use solidcraft_engine::bug_report::{self, Clear};

/// The command's usage, from the same lines the top-level usage lists.
fn usage() -> String {
    crate::command_usage("reports")
}

/// The value after `flag` in `args`.
fn after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
}

pub fn run(args: &[String]) -> Result<(), String> {
    let dir = bug_report::default_dir().ok_or("no report folder on this system")?;
    let id = || args.get(1).map(String::as_str).ok_or_else(usage);
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let want = if args.iter().any(|a| a == "--open") {
                Some("open")
            } else if args.iter().any(|a| a == "--fixed") {
                Some("fixed")
            } else {
                None
            };
            let reports: Vec<_> = bug_report::list(&dir).into_iter().filter(|r| want.is_none_or(|w| r.status == w)).collect();
            if args.iter().any(|a| a == "--json") {
                println!("{}", super::pretty(&json!({ "dir": dir.to_string_lossy(), "reports": reports })));
                return Ok(());
            }
            println!("bug reports in {}", dir.display());
            if reports.is_empty() {
                println!("  (none)");
            }
            for r in reports {
                let issue = r.issue.map(|n| format!("  #{n}")).unwrap_or_default();
                let commit = r.commit.map(|c| format!("  fixed in {c}")).unwrap_or_default();
                let replay = if r.replayable { "  [replayable]" } else { "" };
                println!("  {}  {}{issue}{commit}  v{}{replay}\n      {}", r.id, r.status, r.version, r.title);
            }
            Ok(())
        }
        Some("show") => {
            println!("{}", super::pretty(&bug_report::read(&dir, id()?)?));
            Ok(())
        }
        Some("replay") => replay(&bug_report::read(&dir, id()?)?),
        Some("status") => {
            let id = id()?;
            let state = args.get(2).map(String::as_str).filter(|s| !s.starts_with("--"));
            let issue = match after(args, "--issue") {
                Some(n) => Some(n.trim_start_matches('#').parse::<u64>().map_err(|_| format!("--issue `{n}` must be an issue number"))?),
                None => None,
            };
            let v = bug_report::set_status(&dir, id, state, issue, after(args, "--commit"))?;
            println!("{id}: {}", super::pretty(&v["status"]));
            Ok(())
        }
        Some("issue-text") => {
            let (title, body) = issue_text(&bug_report::read(&dir, id()?)?);
            if args.iter().any(|a| a == "--json") {
                println!("{}", super::pretty(&json!({ "title": title, "body": body })));
            } else {
                println!("{title}\n\n{body}");
            }
            Ok(())
        }
        Some("dir") => {
            println!("{}", dir.display());
            Ok(())
        }
        Some("clear") => {
            let which = match args.get(1).map(String::as_str) {
                Some("--fixed") => Clear::Fixed,
                Some(id) => Clear::One(id),
                None => return Err(usage()),
            };
            println!("deleted {} report(s)", bug_report::clear(&dir, which));
            Ok(())
        }
        _ => Err(usage()),
    }
}

/// Runs a report's commands on its starting design, then the command its dialog would have run,
/// and says how each went and which features fail at the end.
fn replay(report: &Value) -> Result<(), String> {
    let id = report["id"].as_str().unwrap_or_default();
    let (mut s, from) = match (report["replay_path"].as_str(), report["start_path"].as_str(), report["design_path"].as_str()) {
        (Some(_), Some(start), _) => (super::load(start)?, "the design this run started from"),
        (Some(_), None, _) => (solidcraft_engine::Session::default(), "a new design"),
        (None, _, Some(design)) => (super::load(design)?, "the design as it was when reported (no replay)"),
        (None, _, None) => return Err(format!("report {id} has neither a replay nor a design (see its notes)")),
    };
    println!("report {id}: {}", report["title"].as_str().unwrap_or_default());
    if let Some(path) = report["replay_path"].as_str() {
        let commands = super::read_json(path)?["commands"].as_array().cloned().unwrap_or_default();
        println!("replaying {} command(s) on {from}", commands.len());
        for (i, c) in commands.iter().enumerate() {
            let cmd = c["command"].as_str().unwrap_or_default();
            match s.execute(cmd, c.get("params").unwrap_or(&Value::Null)) {
                Ok(_) => println!("  {:>3}. {cmd}: ok", i + 1),
                Err(e) => println!("  {:>3}. {cmd}: {e}", i + 1),
            }
        }
    } else {
        println!("starting from {from}");
    }
    match report["dialog_commands"].as_array() {
        Some(cmds) if !cmds.is_empty() => {
            println!("the open dialog's command(s):");
            for c in cmds {
                let cmd = c["command"].as_str().unwrap_or_default();
                match s.execute(cmd, c.get("params").unwrap_or(&Value::Null)) {
                    Ok(_) => println!("  {cmd}: ok"),
                    Err(e) => println!("  {cmd}: {e}"),
                }
            }
        }
        _ => println!("(no dialog was open)"),
    }
    let failing: Vec<String> =
        s.doc.features.iter().filter_map(|f| s.model.result(f.id).and_then(|r| r.error.as_deref()).map(|e| format!("  {}: {e}", f.name))).collect();
    if failing.is_empty() {
        println!("no feature fails now");
    } else {
        println!("failing features now:\n{}", failing.join("\n"));
    }
    let then: Vec<String> = report["design"]["failing_features"]
        .as_array()
        .map(|a| a.iter().map(|f| format!("  {}: {}", f["name"].as_str().unwrap_or_default(), f["error"].as_str().unwrap_or_default())).collect())
        .unwrap_or_default();
    if !then.is_empty() {
        println!("failing when reported:\n{}", then.join("\n"));
    }
    Ok(())
}

/// The report as a GitHub issue, safe to publish: what the user wrote, the failing features, the
/// dialog's command, the app and system and the warnings from the log, with home folders and the
/// user name taken out. The design, the log and the screenshot stay on this computer.
pub fn issue_text(report: &Value) -> (String, String) {
    let scrub = scrubber();
    let s = |k: &str| scrub(report[k].as_str().unwrap_or_default().trim());
    let section = |head: &str, text: String| if text.is_empty() { String::new() } else { format!("## {head}\n{text}\n\n") };
    let id = report["id"].as_str().unwrap_or_default();
    let failing: Vec<String> = report["design"]["failing_features"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|f| format!("- {}: {}", scrub(f["name"].as_str().unwrap_or_default()), scrub(f["error"].as_str().unwrap_or_default())))
                .collect()
        })
        .unwrap_or_default();
    let dialog = match &report["dialog_commands"] {
        Value::Null => String::new(),
        d => format!("```json\n{}\n```", scrub(&super::pretty(d))),
    };
    let warnings: Vec<String> = report["log_path"]
        .as_str()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|log| {
            let w: Vec<&str> = log.lines().filter(|l| l.contains(" warn ") || l.contains(" error ")).collect();
            w[w.len().saturating_sub(20)..].iter().map(|l| scrub(l)).collect()
        })
        .unwrap_or_default();
    let app = &report["app"];
    let system = &report["system"];
    let mut body = String::new();
    body.push_str(&section("What happened", s("what_happened")));
    body.push_str(&section("Expected", s("expected")));
    body.push_str(&section("Steps", s("steps")));
    body.push_str(&section("Failing features", failing.join("\n")));
    body.push_str(&section("The open dialog's command", dialog));
    body.push_str(&section(
        "Warnings and errors in the log (last 20)",
        if warnings.is_empty() { String::new() } else { format!("```\n{}\n```", warnings.join("\n")) },
    ));
    body.push_str(&section(
        "Environment",
        format!(
            "SolidCraft {} ({}, built {}) on {} {}\n{}",
            app["version"].as_str().unwrap_or_default(),
            app["commit"].as_str().unwrap_or_default(),
            app["built"].as_str().unwrap_or_default(),
            system["os"].as_str().unwrap_or_default(),
            system["arch"].as_str().unwrap_or_default(),
            scrub(system["environment"].as_str().unwrap_or_default())
        ),
    ));
    body.push_str(&format!(
        "The full report (design, replay, log, screenshot) stays on the reporter's computer as `{id}`: `solidcraft-cli reports replay {id}`.\n\n<!-- solidcraft-report: {id} -->\n"
    ));
    let title = s("title");
    (if title.is_empty() { format!("Bug report {id}") } else { title }, body)
}

/// Takes the home folder and the user name out of text that leaves this computer.
fn scrubber() -> impl Fn(&str) -> String {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok().filter(|h| h.len() > 1);
    let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).ok().filter(|u| u.len() > 2);
    move |text: &str| {
        let mut t = text.to_string();
        if let Some(h) = &home {
            t = t.replace(h.as_str(), "~");
        }
        if let Some(u) = &user {
            t = t.replace(u.as_str(), "<user>");
        }
        t
    }
}
