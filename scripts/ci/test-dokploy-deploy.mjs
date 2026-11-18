#!/usr/bin/env node
// Tests for scripts/ci/dokploy-deploy.sh against an in-process MOCK Dokploy API, GitHub API and
// health endpoints, plus static checks of the release jobs in .github/workflows/ci.yml. Nothing
// here contacts Dokploy, GitHub or production.
//
//   node scripts/ci/test-dokploy-deploy.mjs     (needs bash, curl and jq; CI runs it on Ubuntu)
//
// `release()` runs the same step sequence as the `deploy` job: preflight → backend deploy →
// backend verify → frontend deploy → frontend + backend verify, and the frontend rollback step
// when the job fails after the frontend was touched.

import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = join(HERE, "dokploy-deploy.sh");
const WORKFLOW = join(HERE, "..", "..", ".github", "workflows", "ci.yml");
const KEY = "test-api-key-must-never-be-printed-0123456789";
const GH_TOKEN = "test-github-token-must-never-be-printed";
// Commit history of main, oldest first; DIVERGED is a commit that is not on that line.
const C = ["1111", "2222", "3333", "4444"].map((p) => p.repeat(10));
const DIVERGED = "dddd".repeat(10);
const RELEASE = C[2];
const PREVIOUS = C[1];
const DIGEST = "ab".repeat(32);
const BACKEND_APP_NAME = "hourwise-backend-abc123";
const image = (name, sha) => `ghcr.io/owner/hourwise-${name}:sha-${sha}@sha256:${DIGEST}`;

function mock(options = {}) {
  const o = {
    deployedRevision: PREVIOUS, // what production runs before the release
    sourceType: "docker",
    autoDeploy: false,
    updateConfigSwarm: null, // null = Dokploy's default (start-first + rollback)
    frontendHealthCheckSwarm: null,
    deployResult: { backend: "done", frontend: "done" },
    rollsBack: {}, // app -> true: the release's container never gets healthy, Swarm keeps the old one
    stuckRunning: false,
    neverStarts: false,
    brokenProxy: false, // the release's frontend serves, but can't reach the backend
    healthStatus: {}, // app -> HTTP status of its health endpoint
    shapes: {}, // procedure -> "wrapped" | "array" | "object" | "html"
    failFirstRequests: 0, // transient 503s
    ...options,
  };
  const calls = [];
  const apps = {};
  for (const name of ["backend", "frontend"]) {
    apps[`id-${name}`] = {
      name,
      appName: name === "backend" ? BACKEND_APP_NAME : "hourwise-frontend-def456",
      sourceType: o.sourceType,
      autoDeploy: o.autoDeploy,
      dockerImage: o.deployedRevision ? image(name, o.deployedRevision) : "",
      applicationStatus: "done",
      updateConfigSwarm: o.updateConfigSwarm,
      healthCheckSwarm: name === "frontend" ? o.frontendHealthCheckSwarm : null,
      env: "DATABASE_URL=postgres://secret-in-dokploy-env", // never printed
      serving: o.deployedRevision ?? "unknown",
      deployCount: 0,
      deployments: [],
    };
  }
  let failures = o.failFirstRequests;
  let seq = 0;

  const server = createServer((req, res) => {
    const url = new URL(req.url, "http://mock");
    let body = "";
    req.on("data", (c) => (body += c));
    req.on("end", () => {
      const send = (status, json, raw) => {
        res.writeHead(status, { "Content-Type": raw ? "text/html" : "application/json" });
        res.end(raw ?? JSON.stringify(json));
      };
      const path = url.pathname;
      // https://<app>/healthz is the frontend; https://<app>/api/health/ready is the backend through the frontend proxy.
      if (path === "/healthz" || path === "/api/health/ready") {
        const which = path === "/healthz" ? "frontend" : "backend";
        const app = apps[`id-${which}`];
        const status = o.healthStatus[which] ?? 200;
        if (status !== 200) return send(status, { status: "unavailable" });
        if (which === "backend" && o.brokenProxy && apps["id-frontend"].serving === RELEASE) {
          return send(502, null, "<html>Bad Gateway</html>");
        }
        return send(200, { status: "ok", revision: app.serving });
      }
      if (path.startsWith("/gh/")) {
        if (req.headers.authorization !== `Bearer ${GH_TOKEN}`) return send(401, {});
        const m = path.match(/\/compare\/(\w+)\.\.\.(\w+)$/);
        const idx = (ref) => (ref === "main" ? C.length - 1 : C.indexOf(ref));
        if ([m[1], m[2]].includes(DIVERGED)) {
          return send(200, { status: m[1] === m[2] ? "identical" : "diverged" });
        }
        const [a, b] = [idx(m[1]), idx(m[2])];
        if (a < 0 || b < 0) return send(404, { message: "Not Found" });
        return send(200, { status: a === b ? "identical" : b > a ? "ahead" : "behind" });
      }
      // `apiKey` simulates Dokploy expecting a different key than the one configured.
      if (req.headers["x-api-key"] !== (o.apiKey ?? KEY)) return send(401, { message: "Unauthorized" });
      if (failures > 0) {
        failures -= 1;
        return send(503, { message: "busy" });
      }
      const input = req.method === "GET" ? Object.fromEntries(url.searchParams) : JSON.parse(body || "{}");
      const app = apps[input.applicationId];
      const proc = path.replace(/^\/api\//, "");
      calls.push(`${proc}:${app?.name ?? input.applicationId}`);
      if (!app) return send(404, { message: "Application not found" });
      const shaped = (json) => {
        switch (o.shapes[proc]) {
          case "wrapped":
            return send(200, { result: { data: json } });
          case "array":
            return send(200, [json]);
          case "object":
            return send(200, { deployments: json });
          case "html":
            return send(200, null, "<!doctype html><title>Login</title>");
          default:
            return send(200, json);
        }
      };
      switch (proc) {
        case "application.one": {
          const { deployments: _d, serving: _s, deployCount: _c, ...rest } = app;
          return shaped({ applicationId: input.applicationId, ...rest });
        }
        case "application.update":
          if (input.dockerImage) app.dockerImage = input.dockerImage;
          return send(200, true);
        case "application.deploy": {
          if (o.neverStarts) return send(200, true);
          const revision = app.dockerImage.match(/:sha-(\w{40})@/)[1];
          app.deployCount += 1;
          const first = app.deployCount === 1; // the release; later ones are rollbacks
          setTimeout(() => {
            const dep = {
              deploymentId: `dep-${++seq}`,
              title: input.title,
              description: input.description,
              status: "running",
              createdAt: new Date().toISOString(),
            };
            app.deployments.unshift(dep);
            app.applicationStatus = "running";
            if (o.stuckRunning) return;
            setTimeout(() => {
              const result = first ? o.deployResult[app.name] : "done";
              dep.status = result;
              app.applicationStatus = result === "done" ? "done" : "error";
              if (result === "error") dep.errorMessage = "docker pull: manifest unknown";
              if (result === "done" && !(first && o.rollsBack[app.name])) {
                setTimeout(() => (app.serving = revision), 300);
              }
            }, 400);
          }, 300);
          return send(200, true);
        }
        case "deployment.all":
          // A deployment of some other run, to make sure it is not mistaken for ours.
          return shaped([...app.deployments, { deploymentId: "old", description: "Manual deployment", status: "done" }]);
        default:
          return send(404, { message: `unknown procedure ${proc}` });
      }
    });
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      resolve({ server, calls, apps, base: `http://127.0.0.1:${server.address().port}` });
    });
  });
}

function run(args, env) {
  return new Promise((resolve) => {
    const child = spawn("bash", [SCRIPT, ...args], { env: { ...process.env, ...env } });
    let out = "";
    child.stdout.on("data", (d) => (out += d));
    child.stderr.on("data", (d) => (out += d));
    child.on("close", (code) => resolve({ code, out }));
  });
}

function baseEnv(m, dir) {
  return {
    DOKPLOY_URL: `${m.base}/`,
    DOKPLOY_API_KEY: KEY,
    DOKPLOY_BACKEND_APP_ID: "id-backend",
    DOKPLOY_FRONTEND_APP_ID: "id-frontend",
    PRODUCTION_URL: m.base,
    PRODUCTION_BACKEND_URL: `http://${BACKEND_APP_NAME}:8080`,
    RELEASE_SHA: RELEASE,
    GITHUB_TOKEN: GH_TOKEN,
    GITHUB_REPOSITORY: "owner/hourwise",
    GITHUB_API_URL: `${m.base}/gh`,
    GITHUB_RUN_ID: "42",
    GITHUB_RUN_ATTEMPT: "1",
    GITHUB_OUTPUT: join(dir, "output"),
    GITHUB_STEP_SUMMARY: join(dir, "summary"),
    ALLOW_HTTP_URLS: "1",
    POLL_INTERVAL: "0.1",
    DEPLOY_QUEUE_TIMEOUT: "3",
    DEPLOY_TIMEOUT: "3",
    HEALTH_TIMEOUT: "3",
  };
}

function readOutputs(file) {
  return Object.fromEntries(
    readFileSync(file, "utf8")
      .split("\n")
      .filter(Boolean)
      .map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]),
  );
}

function checkNoSecrets(text) {
  for (const secret of [KEY, GH_TOKEN, "secret-in-dokploy-env"]) {
    if (text.includes(secret)) throw new Error(`a secret was printed\n--- log ---\n${text}`);
  }
}

// Mirrors the deploy job's steps. Returns the exit code of the job, the step that failed, whether
// the rollback step ran and succeeded, the log and the job summary.
async function release(m, { env: extra = {}, images = {}, unset = [] } = {}) {
  const dir = mkdtempSync(join(tmpdir(), "hw-deploy-test-"));
  const env = { ...baseEnv(m, dir), ...extra };
  for (const k of unset) delete env[k];
  writeFileSync(env.GITHUB_OUTPUT ?? join(dir, "output"), "");
  const sha = env.RELEASE_SHA;
  const app = m.base;
  const steps = [
    ["preflight"],
    ["deploy", "backend", "id-backend", images.backend ?? image("backend", sha)],
    ["verify", "backend", `${app}/api/health/ready`],
    ["deploy", "frontend", "id-frontend", images.frontend ?? image("frontend", sha)],
    ["verify", "frontend", `${app}/healthz`],
    ["verify", "backend", `${app}/api/health/ready`],
  ];
  let log = "";
  let result = { code: 0, step: "complete", rollback: "not run" };
  for (const [i, args] of steps.entries()) {
    const r = await run(args, env);
    log += r.out;
    if (r.code !== 0) {
      result = { code: r.code, step: args.slice(0, 2).join(" "), rollback: "not run" };
      if (i >= 3) {
        // The workflow's rollback step: only once the frontend has been touched.
        const previous = readOutputs(join(dir, "output")).frontend_previous ?? "";
        const rb = await run(["rollback", "id-frontend", previous, app], env);
        log += rb.out;
        result.rollback = rb.code === 0 ? "succeeded" : "failed";
      }
      break;
    }
    if (i === 0 && readOutputs(join(dir, "output")).decision !== "deploy") {
      result = { code: 0, step: "skipped", rollback: "not run" };
      break;
    }
  }
  let summary = "";
  try {
    summary = readFileSync(join(dir, "summary"), "utf8");
  } catch {
    // no summary written
  }
  rmSync(dir, { recursive: true, force: true });
  checkNoSecrets(log + summary);
  return { ...result, log, summary };
}

const tests = [];
const test = (name, fn) => tests.push({ name, fn });
function assert(cond, msg, ctx) {
  if (!cond) throw new Error(`${msg}\n--- log ---\n${ctx?.log ?? ""}`);
}
const deployCalls = (m) => m.calls.filter((c) => c.startsWith("application.deploy"));
const writes = (m) => m.calls.filter((c) => /application\.(update|deploy)/.test(c));
async function withMock(options, fn) {
  const m = await mock(options);
  try {
    await fn(m);
  } finally {
    m.server.close();
  }
}

// ---- happy paths and ordering -------------------------------------------------------------------

test("deploys backend first, then frontend, and verifies both revisions", () =>
  withMock({}, async (m) => {
    const r = await release(m);
    assert(r.code === 0 && r.step === "complete", `expected success, got ${r.step}`, r);
    assert(
      JSON.stringify(deployCalls(m)) === JSON.stringify(["application.deploy:backend", "application.deploy:frontend"]),
      `wrong deploy order: ${deployCalls(m)}`,
      r,
    );
    assert(m.apps["id-backend"].dockerImage === image("backend", RELEASE), "backend image not pinned", r);
    assert(m.apps["id-frontend"].serving === RELEASE, "frontend not serving the release", r);
    assert(r.summary.includes("serves `333333333333`"), "summary misses the verification", r);
  }));

test("first pipeline release (no CI image deployed yet) is allowed", () =>
  withMock({ deployedRevision: null }, async (m) => {
    const r = await release(m);
    assert(r.code === 0 && r.step === "complete", `expected success, got ${r.step}`, r);
  }));

test("re-running the deployed release redeploys it (identical)", () =>
  withMock({ deployedRevision: RELEASE }, async (m) => {
    const r = await release(m);
    assert(r.code === 0 && r.step === "complete", `expected success, got ${r.step}`, r);
  }));

test("transient 503s from the API are retried", () =>
  withMock({ failFirstRequests: 2 }, async (m) => {
    const r = await release(m);
    assert(r.code === 0 && r.step === "complete", `expected success, got ${r.step}`, r);
    assert(r.log.includes("retrying"), "no retry logged", r);
  }));

// ---- revision order ------------------------------------------------------------------------------

test("an older run never overwrites a newer release", () =>
  withMock({ deployedRevision: C[3] }, async (m) => {
    const r = await release(m);
    assert(r.code === 0 && r.step === "skipped", `expected skip, got ${r.step}`, r);
    assert(writes(m).length === 0, "it changed production", r);
    assert(r.log.includes("newer than"), "no explanation", r);
  }));

test("after a manual rollback to an older release, the next newer commit deploys", () =>
  withMock({ deployedRevision: C[0] }, async (m) => {
    const r = await release(m);
    assert(r.code === 0 && r.step === "complete", `expected success, got ${r.step}`, r);
  }));

test("diverged history (production runs a commit not behind this one) fails without changes", () =>
  withMock({ deployedRevision: DIVERGED }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "preflight" && r.log.includes("not an ancestor"), "expected diverged error", r);
    assert(writes(m).length === 0, "it changed production", r);
  }));

test("refuses a commit that is no longer on main", () =>
  withMock({}, async (m) => {
    const r = await release(m, { env: { RELEASE_SHA: "9".repeat(40) } });
    assert(r.code !== 0 && r.step === "preflight" && r.log.includes("no longer on main"), "unclear error", r);
    assert(writes(m).length === 0, "deployed anyway", r);
  }));

test("images must be pinned to the release commit by digest", () =>
  withMock({}, async (m) => {
    const r = await release(m, { images: { backend: `ghcr.io/owner/hourwise-backend:sha-${RELEASE}` } });
    assert(r.code !== 0 && r.step === "deploy backend" && r.log.includes("not pinned"), "expected pin error", r);
    assert(writes(m).length === 0, "deployed an unpinned image", r);
  }));

test("an image of a different commit is refused", () =>
  withMock({}, async (m) => {
    const r = await release(m, { images: { backend: image("backend", C[3]) } });
    assert(r.code !== 0 && r.step === "deploy backend" && r.log.includes("not pinned"), "expected pin error", r);
    assert(writes(m).length === 0, "deployed the wrong commit", r);
  }));

// ---- configuration -------------------------------------------------------------------------------

for (const name of [
  "DOKPLOY_URL",
  "DOKPLOY_API_KEY",
  "DOKPLOY_BACKEND_APP_ID",
  "DOKPLOY_FRONTEND_APP_ID",
  "PRODUCTION_URL",
  "PRODUCTION_BACKEND_URL",
  "GITHUB_TOKEN",
  "RELEASE_SHA",
]) {
  test(`missing ${name} fails the preflight before any change`, () =>
    withMock({}, async (m) => {
      const r = await release(m, { unset: [name] });
      assert(r.code !== 0 && r.step === "preflight" && r.log.includes(`${name} is not set`), "expected missing-variable error", r);
      assert(writes(m).length === 0, "it changed production", r);
    }));
}

test("PRODUCTION_BACKEND_URL form is validated (publish job)", async () => {
  const dir = mkdtempSync(join(tmpdir(), "hw-deploy-test-"));
  try {
    for (const bad of ["", "https://hourwise-backend-abc:8080", "http://Hourwise:8080", "http://backend", "http://a b:8080", "http://x:8080/api"]) {
      const r = await run(["validate-backend-url"], { PRODUCTION_BACKEND_URL: bad });
      assert(r.code !== 0, `accepted ${JSON.stringify(bad)}`, r);
    }
    const ok = await run(["validate-backend-url"], { PRODUCTION_BACKEND_URL: `http://${BACKEND_APP_NAME}:8080` });
    assert(ok.code === 0, "rejected a valid URL", ok);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a PRODUCTION_BACKEND_URL that isn't the backend's Dokploy service fails before any change", () =>
  withMock({}, async (m) => {
    const r = await release(m, { env: { PRODUCTION_BACKEND_URL: "http://hourwise-backend-old999:8080" } });
    assert(r.code !== 0 && r.step === "preflight" && r.log.includes("service name is"), "expected service-name error", r);
    assert(writes(m).length === 0, "it changed production", r);
  }));

test("PRODUCTION_URL must be a bare https origin", () =>
  withMock({}, async (m) => {
    let r = await release(m, { env: { ALLOW_HTTP_URLS: "" } });
    assert(r.code !== 0 && r.log.includes("must be an https:// URL"), "http accepted", r);
    r = await release(m, { env: { PRODUCTION_URL: `${m.base}/app` } });
    assert(r.code !== 0 && r.log.includes("no path"), "a URL with a path was accepted", r);
    assert(writes(m).length === 0, "it changed production", r);
  }));

test("refuses apps still on the GitHub (branch) source", () =>
  withMock({ sourceType: "github" }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "preflight" && r.log.includes("Docker provider"), "expected source error", r);
  }));

test("refuses apps with Dokploy Auto Deploy enabled", () =>
  withMock({ autoDeploy: true }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "preflight" && r.log.includes("Auto Deploy"), "expected auto-deploy error", r);
  }));

test("refuses a Swarm update config without start-first + rollback", () =>
  withMock({ updateConfigSwarm: { Parallelism: 1, Order: "stop-first", FailureAction: "pause" } }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "preflight" && r.log.includes("start-first"), "expected update-config error", r);
  }));

test("accepts an explicit start-first + rollback update config", () =>
  withMock({ updateConfigSwarm: { Parallelism: 1, Order: "start-first", FailureAction: "rollback" } }, async (m) => {
    const r = await release(m);
    assert(r.code === 0 && r.step === "complete", `expected success, got ${r.step}`, r);
  }));

test("refuses a frontend Swarm health check that bypasses the backend gate", () =>
  withMock({ frontendHealthCheckSwarm: { Test: ["CMD", "node", "-e", "fetch('http://127.0.0.1:3000/healthz')"] } }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "preflight" && r.log.includes("healthcheck.mjs"), "expected health-check error", r);
    assert(writes(m).length === 0, "it changed production", r);
  }));

test("a rejected API key fails clearly", () =>
  withMock({ apiKey: "a-different-key" }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "preflight" && r.log.includes("HTTP 401") && r.log.includes("read, edit and deploy"), "expected 401 failure", r);
  }));

// ---- unexpected API responses --------------------------------------------------------------------

for (const shape of ["wrapped", "array", "html"]) {
  test(`application.one returning ${shape} fails the preflight before any change`, () =>
    withMock({ shapes: { "application.one": shape } }, async (m) => {
      const r = await release(m);
      assert(r.code !== 0 && r.step === "preflight", "expected preflight failure", r);
      assert(/unexpected structure|not JSON/.test(r.log), "unclear error", r);
      assert(writes(m).length === 0, "it changed production", r);
    }));
}

test("deployment.all returning an object instead of a list fails the backend step", () =>
  withMock({ shapes: { "deployment.all": "object" } }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "deploy backend" && r.log.includes("not a list"), "expected structure error", r);
    assert(!deployCalls(m).includes("application.deploy:frontend"), "frontend deployed", r);
  }));

// ---- failed deployments and timeouts -------------------------------------------------------------

test("a failed backend deployment fails the job and never deploys the frontend", () =>
  withMock({ deployResult: { backend: "error", frontend: "done" } }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "deploy backend", `expected backend failure, got ${r.step}`, r);
    assert(r.log.includes("manifest unknown"), "error message not reported", r);
    assert(!deployCalls(m).includes("application.deploy:frontend") && r.rollback === "not run", "frontend touched", r);
  }));

test("a deployment that never starts times out", () =>
  withMock({ neverStarts: true }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "deploy backend" && r.log.includes("did not start"), "expected timeout", r);
  }));

test("a deployment stuck in 'running' times out", () =>
  withMock({ stuckRunning: true }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "deploy backend" && r.log.includes("still running after"), "expected timeout", r);
    assert(!deployCalls(m).includes("application.deploy:frontend"), "frontend deployed", r);
  }));

test("'done' is not trusted: a backend that never serves the release fails verification", () =>
  withMock({ rollsBack: { backend: true } }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "verify backend", `expected verify failure, got ${r.step}`, r);
    assert(r.log.includes(`revision '${PREVIOUS}'`), "doesn't say which revision is served", r);
    assert(!deployCalls(m).includes("application.deploy:frontend"), "frontend deployed", r);
  }));

test("a backend health endpoint answering 503 fails verification", () =>
  withMock({ healthStatus: { backend: 503 } }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "verify backend" && r.log.includes("HTTP 503"), "expected 503 failure", r);
    assert(!deployCalls(m).includes("application.deploy:frontend"), "frontend deployed", r);
  }));

// ---- frontend failures and automatic rollback ----------------------------------------------------

test("a failed frontend deployment fails the job and restores the previous frontend", () =>
  withMock({ deployResult: { backend: "done", frontend: "error" } }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "deploy frontend", `expected frontend failure, got ${r.step}`, r);
    assert(r.rollback === "succeeded", `rollback ${r.rollback}`, r);
    assert(m.apps["id-frontend"].dockerImage === image("frontend", PREVIOUS), "Dokploy still points at the failed image", r);
  }));

test("frontend that can't reach the backend: Swarm keeps the old one, the job fails and re-pins it", () =>
  withMock({ rollsBack: { frontend: true } }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "verify frontend", `expected verify failure, got ${r.step}`, r);
    assert(r.rollback === "succeeded", `rollback ${r.rollback}`, r);
    assert(m.apps["id-frontend"].serving === PREVIOUS, "old frontend not serving", r);
    assert(m.apps["id-frontend"].dockerImage === image("frontend", PREVIOUS), "Dokploy still points at the bad image", r);
    assert(m.apps["id-backend"].dockerImage === image("backend", RELEASE), "backend must not be rolled back", r);
  }));

test("frontend that serves but whose proxy is broken is detected and rolled back", () =>
  withMock({ brokenProxy: true }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "verify backend", `expected backend-through-frontend failure, got ${r.step}`, r);
    assert(r.log.includes("HTTP 502"), "502 not reported", r);
    assert(r.rollback === "succeeded", `rollback ${r.rollback}`, r);
    assert(m.apps["id-frontend"].serving === PREVIOUS, "broken frontend still serving", r);
  }));

test("a frontend answering 503 fails verification; the rollback reports it cannot confirm recovery", () =>
  withMock({ healthStatus: { frontend: 503 } }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.step === "verify frontend" && r.log.includes("HTTP 503"), "expected 503 failure", r);
    // The mocked endpoint stays at 503, so the rollback's own check fails too, and says so.
    assert(r.rollback === "failed" && r.log.includes("health check"), "rollback outcome unclear", r);
  }));

test("rollback refuses when there is no pinned previous frontend image", () =>
  withMock({ deployedRevision: null, rollsBack: { frontend: true } }, async (m) => {
    const r = await release(m);
    assert(r.code !== 0 && r.rollback === "failed" && r.log.includes("Roll back by hand"), "expected manual-rollback message", r);
  }));

// ---- workflow (static) -------------------------------------------------------------------------

function job(yaml, name) {
  const start = yaml.indexOf(`\n  ${name}:\n`);
  if (start < 0) throw new Error(`job ${name} not found`);
  const rest = yaml.slice(start + 1);
  const next = rest.slice(3).search(/\n {2}[a-z][\w-]*:\n/);
  return next < 0 ? rest : rest.slice(0, next + 3);
}

test("workflow: deploy job timeout exceeds the sum of its step timeouts", async () => {
  const deploy = job(readFileSync(WORKFLOW, "utf8"), "deploy");
  const jobTimeout = Number(deploy.match(/\n {4}timeout-minutes: (\d+)/)[1]);
  const steps = deploy.split(/\n {6}- /).slice(1);
  const limits = steps.map((s) => s.match(/\n {8}timeout-minutes: (\d+)/)?.[1]);
  assert(limits.every(Boolean), `every deploy step needs timeout-minutes (${limits})`);
  const sum = limits.reduce((a, b) => a + Number(b), 0);
  assert(sum < jobTimeout, `steps add up to ${sum} min, job allows ${jobTimeout}`);
});

test("workflow: unset PRODUCTION_PIPELINE disables publish and deploy; PRs never release", async () => {
  const yaml = readFileSync(WORKFLOW, "utf8");
  for (const [name, needle] of [
    ["publish", "(vars.PRODUCTION_PIPELINE == 'publish' || vars.PRODUCTION_PIPELINE == 'deploy')"],
    ["deploy", "vars.PRODUCTION_PIPELINE == 'deploy'"],
  ]) {
    const j = job(yaml, name).replace(/\s+/g, " ");
    assert(j.includes(needle.replace(/\s+/g, " ")), `${name}: PRODUCTION_PIPELINE gate missing`);
    assert(j.includes("github.event_name == 'push' && github.ref == 'refs/heads/main'"), `${name}: main-push gate missing`);
  }
  assert(/\n {4}needs: \[backend, images, frontend, e2e, deploy-script\]/.test(job(yaml, "publish")), "publish must need every check");
  assert(/\n {4}needs: publish\n/.test(job(yaml, "deploy")), "deploy must need publish");
  assert(!/workflow_dispatch|pull_request_target|repository_dispatch|schedule:/.test(yaml), "extra triggers present");
});

test("workflow: Dokploy secrets only in the deploy job, which uses the production environment", async () => {
  const yaml = readFileSync(WORKFLOW, "utf8");
  const deploy = job(yaml, "deploy");
  assert(/\n {4}environment:\n {6}name: production\n/.test(deploy), "deploy must use environment 'production'");
  const outside = yaml.replace(deploy, "");
  assert(!/secrets\.DOKPLOY_/.test(outside), "DOKPLOY secrets used outside the deploy job");
  for (const s of ["DOKPLOY_URL", "DOKPLOY_API_KEY", "DOKPLOY_BACKEND_APP_ID", "DOKPLOY_FRONTEND_APP_ID"]) {
    assert(deploy.includes(`secrets.${s}`), `${s} not used`);
  }
  assert(/\n {4}permissions:\n {6}contents: read\n {4}env:/.test(deploy), "deploy permissions must be contents: read only");
  assert(/concurrency:\n {6}group: production-deploy\n {6}cancel-in-progress: false/.test(deploy), "concurrency guard missing");
  assert(/^permissions:\n {2}contents: read$/m.test(yaml), "workflow default permissions must be contents: read");
});

let failed = 0;
for (const t of tests) {
  try {
    const start = Date.now();
    await t.fn();
    console.log(`ok   ${t.name} (${Date.now() - start} ms)`);
  } catch (e) {
    failed += 1;
    console.log(`FAIL ${t.name}\n${e.message}\n`);
  }
}
console.log(`\n${tests.length - failed} passed, ${failed} failed`);
process.exit(failed ? 1 : 0);
