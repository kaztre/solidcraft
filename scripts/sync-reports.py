#!/usr/bin/env python3
"""Keeps the bug reports saved on this computer and the GitHub issues that track them in step.

    scripts/sync-reports.py [--repo OWNER/NAME] [--dry-run]

- Every open report without an issue is filed as one, with `solidcraft-cli reports issue-text`:
  what the user wrote, the failing features, the dialog's command, the log's warnings and the
  environment, never the design, the log or the screenshot, with home folders and the user name
  removed. The report records the issue's number.
- Every open report whose issue is closed is marked fixed, with the commit whose message says
  "Fixes #N" when git has one.

The repository defaults to the `origin` remote's. Needs `gh` signed in, and `solidcraft-cli`:
$SOLIDCRAFT_CLI, else target/release, else the installed app, else PATH. The pre-push hook runs
it (see docs/crash-reports.md); it never stops a push.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def cli() -> str:
    candidates = [
        os.environ.get("SOLIDCRAFT_CLI", ""),
        str(ROOT / "target" / "release" / "solidcraft-cli"),
        str(Path.home() / "Applications" / "SolidCraft Builds" / "SolidCraft.app" / "Contents" / "MacOS" / "solidcraft-cli"),
        shutil.which("solidcraft-cli") or "",
    ]
    for c in candidates:
        if c and os.access(c, os.X_OK):
            return c
    sys.exit("sync-reports: no solidcraft-cli found (build it, or set SOLIDCRAFT_CLI)")


def run(args: list[str], **kw) -> str:
    return subprocess.run(args, check=True, capture_output=True, text=True, **kw).stdout


def origin_repo() -> str:
    url = run(["git", "-C", str(ROOT), "remote", "get-url", "origin"]).strip()
    m = re.search(r"github\.com[:/]([^/]+/[^/]+?)(?:\.git)?$", url)
    if not m:
        sys.exit(f"sync-reports: origin ({url}) is not a GitHub repository; pass --repo")
    return m.group(1)


def fixing_commit(issue: int) -> str | None:
    out = run(["git", "-C", str(ROOT), "log", "-1", "--format=%h", "-E", f"--grep=(Fix|Fixes|Fixed|Close|Closes|Closed|Resolve|Resolves|Resolved) #{issue}\\b"])
    return out.strip() or None


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--repo")
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()
    if not shutil.which("gh"):
        sys.exit("sync-reports: gh is not installed")
    repo = a.repo or origin_repo()
    sc = cli()
    reports = json.loads(run([sc, "reports", "list", "--open", "--json"]))["reports"]
    if not reports:
        print("sync-reports: no open bug reports")
        return 0
    for r in reports:
        rid, issue = r["id"], r.get("issue")
        if issue is None:
            text = json.loads(run([sc, "reports", "issue-text", rid, "--json"]))
            if a.dry_run:
                print(f"sync-reports: would file {rid} on {repo}: {text['title']}")
                continue
            with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as body:
                body.write(text["body"])
            try:
                url = run(["gh", "issue", "create", "-R", repo, "--title", text["title"], "--body-file", body.name]).strip()
            finally:
                os.unlink(body.name)
            number = int(url.rstrip("/").rsplit("/", 1)[-1])
            run([sc, "reports", "status", rid, "--issue", str(number)])
            print(f"sync-reports: filed {rid} as {url}")
            continue
        state = run(["gh", "issue", "view", str(issue), "-R", repo, "--json", "state", "--jq", ".state"]).strip()
        if state != "CLOSED":
            print(f"sync-reports: {rid} is open as #{issue}")
            continue
        commit = fixing_commit(issue)
        if a.dry_run:
            print(f"sync-reports: would mark {rid} fixed (#{issue} is closed{', by ' + commit if commit else ''})")
            continue
        args = [sc, "reports", "status", rid, "fixed"] + (["--commit", commit] if commit else [])
        run(args)
        print(f"sync-reports: {rid} fixed (#{issue} closed{', by ' + commit if commit else ''})")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except subprocess.CalledProcessError as e:
        sys.exit(f"sync-reports: {' '.join(e.cmd[:3])}… failed: {(e.stderr or '').strip()}")
