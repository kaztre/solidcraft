//! Work on files SolidCraft did not make, run in a helper process. SMLib is C++: a crash inside
//! it on one bad file must fail that file, not close the window with the open design (kaztre#12:
//! a use-after-free in SMLib's stitch of a STEP solid's loose faces took the app down while the
//! Parts Library converted a folder). The app turns helpers on ([`enable`]) and is its own helper
//! (`solidcraft --helper`, [`serve`]); elsewhere (tests, the CLI, the web app) the work runs in
//! this process, as before.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

/// The app's argument that makes a process a helper: it reads the commands from stdin, runs them
/// and writes the reply to stdout ([`serve`]).
pub const ARG: &str = "--helper";

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Run [`run`]'s work in helper processes from now on: this executable serves [`ARG`].
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// Why [`run`]'s work failed, and whether its helper died doing it: a file that crashed a helper
/// must not then be read in the window's own process.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub message: String,
    pub crashed: bool,
}

impl Failure {
    fn new(message: impl Into<String>) -> Self {
        Failure { message: message.into(), crashed: false }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// `commands` (`(id, params)`) run one after another in one fresh session → the last one's reply.
/// In a helper process when helpers are on, so a crash fails only this work; `stop` set (the
/// window closing, the user stopping) ends the helper early.
pub fn run(commands: &[(&str, Value)], stop: &AtomicBool) -> Result<Value, Failure> {
    #[cfg(not(target_arch = "wasm32"))]
    if ENABLED.load(Ordering::Relaxed) {
        return in_helper(commands, stop);
    }
    let _ = stop;
    here(commands).map_err(Failure::new)
}

/// `commands` run in this process.
fn here(commands: &[(&str, Value)]) -> Result<Value, String> {
    let mut s = crate::Session::default();
    let mut reply = Value::Null;
    for (id, params) in commands {
        reply = s.execute(id, params).map_err(|e| e.to_string())?;
    }
    Ok(reply)
}

/// What a helper is sent: the commands, in order.
fn request(commands: &[(&str, Value)]) -> Value {
    Value::Array(commands.iter().map(|(id, params)| json!({"command": id, "params": params})).collect())
}

/// The commands a helper was sent, or why they cannot be read.
fn commands(request: &Value) -> Result<Vec<(String, Value)>, String> {
    let list = request.as_array().ok_or("the helper's request is not a list of commands")?;
    list.iter()
        .map(|c| match (c.get("command").and_then(Value::as_str), c.get("params")) {
            (Some(id), Some(params)) => Ok((id.to_string(), params.clone())),
            _ => Err("a command in the helper's request has no `command` or `params`".to_string()),
        })
        .collect()
}

/// The reply a helper writes for `result`.
fn reply(result: &Result<Value, String>) -> Value {
    match result {
        Ok(v) => json!({"ok": v}),
        Err(e) => json!({"error": e}),
    }
}

/// What a helper's output says: its reply, or why there is none.
fn read_reply(stdout: &[u8], status: &str) -> Result<Value, Failure> {
    let text = String::from_utf8_lossy(stdout);
    let line = text.lines().rev().find(|l| !l.trim().is_empty());
    let parsed: Option<Value> = line.and_then(|l| serde_json::from_str(l).ok());
    match parsed {
        Some(v) if v.get("ok").is_some() => Ok(v["ok"].clone()),
        Some(v) if v.get("error").is_some() => Err(Failure::new(v["error"].as_str().unwrap_or("the helper failed"))),
        _ => Err(Failure {
            message: format!("SolidCraft's geometry kernel stopped while reading this file ({status}); the file was not changed"),
            crashed: true,
        }),
    }
}

/// Serve one request as a helper: the commands from stdin, the reply to stdout. → the process's
/// exit code.
pub fn serve() -> i32 {
    use std::io::{Read, Write};
    let mut input = String::new();
    let result = std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| format!("the helper could not read its request: {e}"))
        .and_then(|_| serde_json::from_str::<Value>(&input).map_err(|e| format!("the helper's request is not JSON: {e}")))
        .and_then(|r| commands(&r))
        .and_then(|list| {
            let borrowed: Vec<(&str, Value)> = list.iter().map(|(id, p)| (id.as_str(), p.clone())).collect();
            here(&borrowed)
        });
    let mut out = std::io::stdout();
    let written = writeln!(out, "{}", reply(&result)).and_then(|_| out.flush());
    match (written, result) {
        (Ok(()), Ok(_)) => 0,
        _ => 1,
    }
}

/// `commands` run by a helper: this executable started with [`ARG`].
#[cfg(not(target_arch = "wasm32"))]
fn in_helper(commands: &[(&str, Value)], stop: &AtomicBool) -> Result<Value, Failure> {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe().map_err(|e| Failure::new(format!("SolidCraft could not find itself to start a helper: {e}")))?;
    let mut child = Command::new(exe)
        .arg(ARG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| Failure::new(format!("SolidCraft could not start a helper: {e}")))?;
    let sent = child.stdin.take().map(|mut stdin| stdin.write_all(request(commands).to_string().as_bytes()));
    // The reply is read beside the wait, so a long one cannot fill the pipe and stall the helper.
    let reader = child.stdout.take().map(|mut stdout| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes);
            bytes
        })
    });
    let status = loop {
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Failure::new("stopped"));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => return Err(Failure::new(format!("the helper could not be waited for: {e}"))),
        }
    };
    let stdout = reader.and_then(|r| r.join().ok()).unwrap_or_default();
    if let Some(Err(e)) = sent
        && stdout.is_empty()
    {
        return Err(Failure::new(format!("the helper could not be sent its work: {e}")));
    }
    read_reply(&stdout, &status.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn a_request_round_trips_and_runs_here_when_helpers_are_off() {
        let list = [("parameters.list", json!({}))];
        let back = commands(&request(&list)).unwrap();
        assert_eq!(back, vec![("parameters.list".to_string(), json!({}))]);
        assert!(run(&list, &AtomicBool::new(false)).is_ok());
        assert!(run(&[("no.such.command", json!({}))], &AtomicBool::new(false)).is_err());
    }

    #[test]
    fn a_helper_that_died_says_so_and_a_reply_is_read_from_its_last_line() {
        assert_eq!(read_reply(b"noise\n{\"ok\":{\"file\":\"a\"}}\n", "exit 0").unwrap(), json!({"file": "a"}));
        assert_eq!(read_reply(b"{\"error\":\"no bodies\"}\n", "exit 1").unwrap_err(), Failure::new("no bodies"));
        let died = read_reply(b"", "signal: 11 (SIGSEGV) (core dumped)").unwrap_err();
        assert!(died.crashed, "{died}");
        assert!(died.message.contains("stopped") && died.message.contains("SIGSEGV") && died.message.contains("not changed"), "{died}");
        for bad in [json!({}), json!([{"command": "parameters.list"}]), json!([{"params": {}}])] {
            assert!(commands(&bad).is_err(), "{bad}");
        }
    }
}
