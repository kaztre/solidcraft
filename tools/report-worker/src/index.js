// SolidCraft report service: files bug reports from a fork's test builds as GitHub issues.
//
// The app (Report a bug > Send report, built with SOLIDCRAFT_REPORT_URL) POSTs the reviewed
// report here as JSON: { title, body, version, os, screenshot_png? (base64) }. This Worker checks
// it, stores the screenshot in R2 (optional), and opens an issue on GITHUB_REPO with a token that
// only it holds. The app never sees the token. See README.md next to this folder.
//
// Bindings (wrangler.jsonc): GITHUB_REPO (var), GITHUB_TOKEN (secret, fine-grained: Issues
// read/write on that one repository and nothing else), REPORT_KEY (secret, optional),
// LIMITER (rate limit, optional), SHOTS (R2 bucket, optional).

const MAX_REQUEST = 10 * 1024 * 1024;
const MAX_TITLE = 160;
const MAX_BODY = 40000;
const MAX_SCREENSHOT = 6 * 1024 * 1024;
const PNG_MAGIC = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const SHOT_PATH = /^\/shots\/([0-9a-f-]{36})\.png$/;

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    try {
      if (request.method === "GET" && SHOT_PATH.test(url.pathname)) {
        return await screenshot(url.pathname.match(SHOT_PATH)[1], env);
      }
      if (request.method === "GET" && url.pathname === "/") {
        return json(200, { service: "SolidCraft report service", repo: env.GITHUB_REPO || null });
      }
      if (url.pathname !== "/" && url.pathname !== "/report") {
        return json(404, { error: "not found" });
      }
      if (request.method !== "POST") {
        return json(405, { error: "use POST" });
      }
      return await report(request, env, url);
    } catch (e) {
      console.error("report service failure", e && e.stack ? e.stack : e);
      return json(500, { error: "the report service failed; please try again later" });
    }
  },
};

async function report(request, env, url) {
  if (!env.GITHUB_TOKEN || !validRepo(env.GITHUB_REPO)) {
    return json(503, { error: "the report service is not set up yet" });
  }
  if (env.REPORT_KEY && request.headers.get("X-Report-Key") !== env.REPORT_KEY) {
    return json(403, { error: "this build cannot send reports here" });
  }
  if (env.LIMITER) {
    const who = request.headers.get("CF-Connecting-IP") || "unknown";
    const { success } = await env.LIMITER.limit({ key: who });
    if (!success) return json(429, { error: "too many reports from here; try again in a minute" });
  }
  const length = Number(request.headers.get("Content-Length") || "0");
  if (length > MAX_REQUEST) return json(413, { error: "the report is too large" });
  const raw = await request.arrayBuffer();
  if (raw.byteLength > MAX_REQUEST) return json(413, { error: "the report is too large" });

  let input;
  try {
    input = JSON.parse(new TextDecoder().decode(raw));
  } catch {
    return json(400, { error: "the report is not JSON" });
  }
  const text = (k, max) => (typeof input[k] === "string" ? input[k].slice(0, max) : "");
  const body = text("body", MAX_BODY);
  if (!body.trim()) return json(400, { error: "the report is empty" });
  const title = oneLine(text("title", MAX_TITLE)) || "Bug report from a test build";
  const version = oneLine(text("version", 64)).replace(/[^0-9A-Za-z.+-]/g, "");
  const os = oneLine(text("os", 64)).replace(/[^0-9A-Za-z _.-]/g, "");

  let shotLine = "";
  if (typeof input.screenshot_png === "string" && input.screenshot_png) {
    const png = decodePng(input.screenshot_png);
    if (!png) {
      shotLine = "_A screenshot was sent but was not a valid PNG, so it was left out._";
    } else if (!env.SHOTS) {
      shotLine = "_A screenshot was sent, but this report service has no storage for screenshots._";
    } else {
      const id = crypto.randomUUID();
      await env.SHOTS.put(`shots/${id}.png`, png, { httpMetadata: { contentType: "image/png" } });
      shotLine = `![Screenshot](${url.origin}/shots/${id}.png)`;
    }
  }

  const issueBody = [
    quiet(body),
    "",
    "---",
    `**Version:** \`${version || "unknown"}\` · **System:** \`${os || "unknown"}\``,
    shotLine,
    "",
    "_Sent from SolidCraft's Report a bug in a test build._",
  ].join("\n");

  const labels = (env.LABELS || "")
    .split(",")
    .map((l) => l.trim())
    .filter(Boolean);
  let issue = await createIssue(env, { title: quiet(title), body: issueBody, labels });
  // A label that does not exist (or a token that may not set labels): file it without them.
  if (issue.status === 422 && labels.length) {
    issue = await createIssue(env, { title: quiet(title), body: issueBody });
  }
  if (issue.status !== 201) {
    console.error("GitHub refused the issue", issue.status, await issue.text());
    return json(502, { error: "GitHub did not accept the report; please try again later" });
  }
  const created = await issue.json();
  return json(201, { url: created.html_url, number: created.number });
}

function createIssue(env, issue) {
  return fetch(`https://api.github.com/repos/${env.GITHUB_REPO}/issues`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${env.GITHUB_TOKEN}`,
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
      "User-Agent": "solidcraft-report-worker",
      "Content-Type": "application/json",
    },
    body: JSON.stringify(issue),
  });
}

async function screenshot(id, env) {
  if (!env.SHOTS) return json(404, { error: "not found" });
  const object = await env.SHOTS.get(`shots/${id}.png`);
  if (!object) return json(404, { error: "not found" });
  return new Response(object.body, {
    headers: {
      "Content-Type": "image/png",
      "Cache-Control": "public, max-age=31536000, immutable",
      "X-Content-Type-Options": "nosniff",
      "Content-Security-Policy": "default-src 'none'",
    },
  });
}

function decodePng(b64) {
  if (b64.length > Math.ceil((MAX_SCREENSHOT * 4) / 3) + 4) return null;
  let bytes;
  try {
    const bin = atob(b64);
    bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  } catch {
    return null;
  }
  if (bytes.length > MAX_SCREENSHOT || !PNG_MAGIC.every((b, i) => bytes[i] === b)) return null;
  return bytes;
}

// Reports are written by anyone with the app: keep them from pinging people (@name) or teams.
function quiet(s) {
  return s.replace(/@(?=[A-Za-z0-9-])/g, "@​");
}

function oneLine(s) {
  return s.replace(/[\r\n\t]+/g, " ").trim();
}

function validRepo(r) {
  return typeof r === "string" && /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(r);
}

function json(status, value) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "Content-Type": "application/json", "Cache-Control": "no-store" },
  });
}
