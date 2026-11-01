# Report service (Cloudflare Worker)

Turns **Report a bug ▸ Send report** in this fork's test builds into GitHub issues on
`kaztre/solidcraft`, without putting a GitHub token in the app. Setup, deployment and the release
side are in [`docs/fork-releases.md`](../../docs/fork-releases.md).

- `src/index.js`: the Worker. `POST /report` with `{title, body, version, os, screenshot_png?}`
  answers `201 {"url": "<issue>", "number": n}` or `{"error": "..."}`; `GET /shots/<id>.png` serves
  a stored screenshot.
- `wrangler.jsonc`: the repository, labels, rate limit (3 a minute per address) and the R2 bucket.
- Secrets (never in files): `GITHUB_TOKEN` (fine-grained, Issues read/write on the one repository),
  `REPORT_KEY` (optional; the app sends it as `X-Report-Key`).

```sh
npx wrangler dev      # local; put GITHUB_TOKEN=... in .dev.vars (ignored by git)
npx wrangler deploy
npx wrangler tail     # live logs
```
