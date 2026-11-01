# Test releases from a fork

How this fork (kaztre/solidcraft, branch `sosub`) gives test builds to people on other computers,
keeps them updated, and gets their bug reports back as GitHub issues.

```
 you ── Run workflow ──► GitHub Actions (fork-release.yml) ──► Release v0.24.0-test.N (latest)
                                                                     │
 tester's app ◄── "Update available" (checks kaztre/solidcraft) ─────┘
 tester's app ── Report a bug ▸ Send report ──► Cloudflare Worker ──► issue on kaztre/solidcraft
                                                (holds the GitHub token)   (+ screenshot in R2)
```

- **Reading** needs nothing special: releases on a public repository are public downloads.
- **Writing** (bug reports) goes through the Worker in `tools/report-worker`, which holds a GitHub
  token that can only create issues on this one repository. The token is never in the app, so a
  tester cannot use it to change the code, releases or anything else.

## What the test builds do differently

Only builds from `.github/workflows/fork-release.yml` behave like this; local and upstream builds
are unchanged.

- Version `<Cargo.toml version>-test.<run number>`, for example `0.24.0-test.12`. Each run is
  higher than the last, also after a rebase onto a newer upstream version.
- The update check asks this repository (`SOLIDCRAFT_UPDATE_REPO`) instead of upstream, and offers
  a newer test build (`crates/ui-egui/src/update.rs`). Testers get **Update now** in the title bar.
- Report a bug's main button is **Send report** (`SOLIDCRAFT_REPORT_URL`). It sends the title, the
  tester's answers, the technical details and the screenshot, all of which the tester sees and can
  edit in the form first. The design file is never sent. A copy is still saved on their computer.

## One-time setup

### 1. GitHub repository settings (github.com/kaztre/solidcraft/settings)

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
4. **Issues ▸ Labels**: add `from-app` (and keep `bug`). Reports get both.

### 2. A GitHub token for the Worker (issues only)

github.com/settings/personal-access-tokens/new (fine-grained):

- Resource owner `kaztre`; **Only select repositories**: `kaztre/solidcraft`.
- Repository permissions: **Issues: Read and write**. Nothing else (Metadata: read is automatic).
- Pick an expiry (a year at most) and note the date; the Worker stops filing when it expires.

Do not use a classic token or one with Contents access: that would let anyone who reached the
Worker's secrets push code.

### 3. Deploy the Worker (Cloudflare, free plan is enough)

```sh
cd tools/report-worker
npx wrangler login                                   # opens the browser once
npx wrangler r2 bucket create solidcraft-report-shots   # screenshots; skip and delete the
                                                     # r2_buckets block in wrangler.jsonc to go without
npx wrangler secret put GITHUB_TOKEN                 # paste the token from step 2
openssl rand -hex 16                                 # a tester key; copy it
npx wrangler secret put REPORT_KEY                   # paste that key
npx wrangler deploy                                  # prints https://solidcraft-reports.<you>.workers.dev
```

Check it (this files a real issue):

```sh
curl -sS https://solidcraft-reports.<you>.workers.dev/report \
  -H 'Content-Type: application/json' -H "X-Report-Key: <the key>" \
  -d '{"title":"Test report","body":"Testing the report service.","version":"0.0.0-test.0","os":"curl"}'
# {"url":"https://github.com/kaztre/solidcraft/issues/1","number":1}
```

Logs: `npx wrangler tail` or the Worker's page in the Cloudflare dashboard.

### 4. Tell the release workflow where the Worker is

github.com/kaztre/solidcraft/settings ▸ Secrets and variables ▸ Actions:

- **Variables ▸ New repository variable** `REPORT_URL` = `https://solidcraft-reports.<you>.workers.dev/report`
- **Secrets ▸ New repository secret** `REPORT_KEY` = the key from step 3

Without them the builds still work; Report a bug then only saves and copies, as upstream's does.

## Cutting a test release

Push `sosub`, then either Actions ▸ **Fork test release** ▸ Run workflow (branch `sosub`), or:

```sh
gh workflow run fork-release.yml --repo kaztre/solidcraft --ref sosub -f notes="Fillet fixes; try the bracket model"
gh run watch --repo kaztre/solidcraft
```

About 30–45 minutes later the release is at github.com/kaztre/solidcraft/releases/latest, and
testers' apps offer it within a day (or at once with Help ▸ Check for updates).

Give new testers that page's link. The fixed names always fetch the newest build:

- macOS: `https://github.com/kaztre/solidcraft/releases/latest/download/solidcraft-macos-universal.dmg`
- Windows: `https://github.com/kaztre/solidcraft/releases/latest/download/solidcraft-windows-x64.msi`
- Linux: `https://github.com/kaztre/solidcraft/releases/latest/download/solidcraft-linux-x86_64.AppImage`

The builds are not code-signed. On macOS the first open needs right-click ▸ Open (or System
Settings ▸ Privacy & Security ▸ Open Anyway); on Windows SmartScreen needs More info ▸ Run anyway.

## Things to know

- **Reports are public.** The fork is public, so issues and the screenshots they link are visible
  to anyone. The form says so; tell testers not to send anything confidential.
- **Spam.** The report URL and key are inside the app, so a determined person could find them.
  The Worker limits each address to 3 reports a minute and refuses requests without the key. If it
  is abused: `npx wrangler secret put REPORT_KEY` with a new key (old builds then stop sending, so
  cut a new release), or delete the Worker.
- **Upstream changes to `release.yml`** are not picked up automatically; `fork-release.yml` copies
  its build steps. If a test release fails after a rebase, compare the two.
- **Rebasing.** `sosub` is rebased onto upstream `develop`; the release numbers keep rising because
  they come from the workflow's run number, not from the commit history.
