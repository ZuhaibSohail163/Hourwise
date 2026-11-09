#!/usr/bin/env node
// Runs the Playwright end-to-end tests against an isolated, throwaway Docker stack.
//
//   npm run test:e2e               # build + start stack, run tests, stop stack
//   E2E_KEEP=1 npm run test:e2e    # leave the stack running afterwards (for debugging)
//   E2E_BASE_URL=http://localhost:3000 npx --prefix frontend playwright test
//                                  # run against an already running app instead (careful:
//                                  # the tests create users/projects in that database)

import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const compose = ["compose", "-f", "docker-compose.e2e.yml", "-p", "hourwise-e2e"];
const baseURL = "http://localhost:3100";

function run(cmd, args, opts = {}) {
  const res = spawnSync(cmd, args, {
    cwd: root,
    stdio: "inherit",
    shell: process.platform === "win32",
    ...opts,
  });
  return res.status ?? 1;
}

async function waitFor(url, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(url);
      if (res.ok) return true;
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  return false;
}

// Always start from a clean database (tmpfs is discarded with the containers).
run("docker", [...compose, "down", "--remove-orphans"]);
if (run("docker", [...compose, "up", "-d", "--build"]) !== 0) process.exit(1);

let status = 1;
try {
  if (!(await waitFor(`${baseURL}/api/health`, 120_000))) {
    console.error("[e2e] the app did not become healthy in time");
    run("docker", [...compose, "logs", "--tail", "50"]);
  } else {
    status = run("npx", ["playwright", "test", ...process.argv.slice(2)], {
      cwd: resolve(root, "frontend"),
      env: { ...process.env, E2E_BASE_URL: baseURL },
    });
  }
} finally {
  if (!process.env.E2E_KEEP) run("docker", [...compose, "down", "--remove-orphans"]);
}
process.exit(status);
