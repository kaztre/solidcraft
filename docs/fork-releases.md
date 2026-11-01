# Test releases from a fork

How this fork (kaztre/solidcraft, branch `sosub`) gives test builds to people on other computers,
keeps them updated, and gets their bug reports back as GitHub issues.

```
 you ── Run workflow ──► GitHub Actions (fork-release.yml) ──► Release v0.24.0-test.N (latest)
                                                                     │
 tester's app ◄── "Update available" (checks kaztre/solidcraft) ─────┘
 tester's app ── Report a bug ▸ Open on GitHub ──► tester's browser ──► issue on kaztre/solidcraft
                                                  (signed in to GitHub)
```

- **Reading** needs nothing special: releases on a public repository are public downloads.
- **Writing** (bug reports) is done by the tester on github.com, signed in with their own GitHub
  account. The app holds no token and uploads nothing; there is no service to run.

## What the test builds do differently

Only builds from `.github/workflows/fork-release.yml` behave like this; local and upstream builds
are unchanged.

- Version `<Cargo.toml version>-test.<run number>`, for example `0.24.0-test.12`. Each run is
  higher than the last, also after a rebase onto a newer upstream version.
- The update check asks this repository (`SOLIDCRAFT_UPDATE_REPO`) instead of upstream, and offers
  a newer test build (`crates/ui-egui/src/update.rs`). Testers get **Update now** in the title bar.
- Report a bug's main button is **Open on GitHub** (`crates/ui-egui/src/issue_report/github.rs`).
  It saves a copy on the tester's computer, then opens a new issue on this repository in their
  browser with the title, their answers and the technical details filled in. They sign in, check
  it and press Submit. The design file is never sent.
  - The screenshot can't be filled in by the address: the app's note says where the saved
    `screenshot.png` is, to drag into the issue.
  - A browser address holds about 8,000 characters. A longer report is cut short there, and the
    whole report is put on the clipboard to paste in instead.

## One-time setup (github.com/kaztre/solidcraft/settings)

1. **General ▸ Features ▸ Issues**: turn on (forks start with issues off).
2. **General ▸ Default branch**: switch to `sosub`. GitHub only lets you run a workflow by hand when
   the workflow file is on the default branch; it also makes the fork's front page show your work.
3. **Actions** tab: enable workflows on the fork. Then turn off upstream's automation, which makes
   no sense here (nightly releases, claims, triage, Pages, store uploads):

   ```sh
   for w in release.yml release-gate.yml claim.yml triage.yml pr-checks.yml pages.yml \
            flatpak-pages.yml snap-store.yml winget.yml docker.yml generated.yml \
            external-corpus.yml freecad-oracle.yml kernel-rig.yml fillet-rig.yml kernel-paused.yml; do
     gh workflow disable "$w" --repo kaztre/solidcraft
   done
   ```

   Keep `fork-release.yml`, and `ci.yml` if you want CI on your pushes.
4. **Issues ▸ Labels**: keep `bug`. The issue address asks for it, but GitHub only applies labels
   for people with write access, so testers' issues arrive unlabelled.

## Cutting a test release

Push `sosub`, then either Actions ▸ **Fork test release** ▸ Run workflow (branch `sosub`), or:

```sh
gh workflow run fork-release.yml --repo kaztre/solidcraft --ref sosub -f notes="Fillet fixes; try the bracket model"
gh run watch --repo kaztre/solidcraft
```

About 30–45 minutes later the release is at github.com/kaztre/solidcraft/releases/latest, and
testers' apps offer it within a day (or at once with Help ▸ Check for updates).

A release holds one download per platform (and `SHA256SUMS.txt`, which the in-app update checks
downloads against), so testers can't pick the wrong file. Give new testers that page's link. The
fixed names always fetch the newest build:

- macOS: `https://github.com/kaztre/solidcraft/releases/latest/download/solidcraft-macos-universal.dmg`
- Windows: `https://github.com/kaztre/solidcraft/releases/latest/download/solidcraft-windows-x64.msi`
- Linux: `https://github.com/kaztre/solidcraft/releases/latest/download/solidcraft-linux-x86_64.AppImage`

The AppImage runs on any distribution (make it executable, then run it) and updates itself in
place. It is built on Ubuntu 24.04 (SMLib's Linux library needs glibc 2.38 or newer), so it needs
Ubuntu 24.04, Fedora 40, Debian 13 or newer. There is no ARM Linux build; Windows is x64 only, and
the macOS dmg runs on both Apple silicon and Intel Macs.

The builds are not code-signed. On macOS the first open needs right-click ▸ Open (or System
Settings ▸ Privacy & Security ▸ Open Anyway); on Windows SmartScreen needs More info ▸ Run anyway.

## Things to know

- **Testers need a GitHub account** (free) to submit a report. Without one they can still use
  Copy report and send it to you some other way.
- **Reports are public.** The fork is public, so issues and their screenshots are visible to
  anyone. The form says so; tell testers not to send anything confidential.
- **Upstream changes to `release.yml`** are not picked up automatically; `fork-release.yml` copies
  its build steps. If a test release fails after a rebase, compare the two.
- **Rebasing.** `sosub` is rebased onto upstream `develop`; the release numbers keep rising because
  they come from the workflow's run number, not from the commit history.
