#!/usr/bin/env node
// Runs `cargo <args>` in ./backend.
// Uses a local Rust toolchain when available, otherwise falls back to the official
// rust Docker image (so contributors without Rust installed can still lint/format).
//
//   node scripts/cargo.mjs fmt --check
//   node scripts/cargo.mjs clippy --all-targets -- -D warnings
//   HOURWISE_CARGO=docker node scripts/cargo.mjs fmt   # force Docker
//
// The opt-in Turso Cloud test settings (HOURWISE_TEST_TURSO_URL / HOURWISE_TEST_TURSO_TOKEN)
// are taken from the shell environment or, failing that, from the repo-root `.env` file, and
// forwarded to cargo. Only those two variables are read from `.env`; the production
// TURSO_DATABASE_URL / TURSO_AUTH_TOKEN are never forwarded to tests.

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const backend = resolve(root, "backend");
const args = process.argv.slice(2);

const TEST_VARS = ["HOURWISE_TEST_TURSO_URL", "HOURWISE_TEST_TURSO_TOKEN"];

/** Minimal .env parser: KEY=value lines, optional `export`, quotes, comments, CRLF and BOM. */
function readDotEnv(file) {
  if (!existsSync(file)) return {};
  const out = {};
  const text = readFileSync(file, "utf8").replace(/^﻿/, "");
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const m = line.match(/^(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$/);
    if (!m) continue;
    let value = m[2];
    const quote = value[0];
    if ((quote === '"' || quote === "'") && value.indexOf(quote, 1) > 0) {
      value = value.slice(1, value.indexOf(quote, 1));
    } else {
      value = value.replace(/\s+#.*$/, "").trim();
    }
    out[m[1]] = value;
  }
  return out;
}

const dotEnv = readDotEnv(resolve(root, ".env"));
const env = { ...process.env };
for (const name of TEST_VARS) {
  if (!env[name] && dotEnv[name]) env[name] = dotEnv[name];
}

// Guard: the test database must never be the production database. Compare hosts, so
// libsql://, https:// and trailing-slash variants of the same database all match.
const normalise = (u) =>
  (u ?? "")
    .trim()
    .replace(/\/+$/, "")
    .toLowerCase()
    .replace(/^[a-z]+:\/\//, "");
const testUrl = normalise(env.HOURWISE_TEST_TURSO_URL);
const prodUrls = [process.env.TURSO_DATABASE_URL, dotEnv.TURSO_DATABASE_URL]
  .filter((u) => u && u.trim())
  .map(normalise);
if (testUrl && prodUrls.includes(testUrl)) {
  console.error(
    "[cargo.mjs] Refusing to run: HOURWISE_TEST_TURSO_URL points at the same database as TURSO_DATABASE_URL.\n" +
      "            Create a separate, empty test database (e.g. `turso db create hourwise-test`).",
  );
  process.exit(1);
}
// The production URL (never its token) is passed to the test as HOURWISE_PROD_TURSO_URL so the
// Rust test can repeat this check itself. TURSO_DATABASE_URL / TURSO_AUTH_TOKEN are not forwarded.
const prodUrl = process.env.TURSO_DATABASE_URL || dotEnv.TURSO_DATABASE_URL;
if (prodUrl) env.HOURWISE_PROD_TURSO_URL = prodUrl;
if (testUrl && args.includes("turso_cloud")) {
  // Only the host is shown; the token is never printed.
  const host = testUrl.split("/")[0];
  console.error(
    `[cargo.mjs] Turso test database: ${host} (token ${env.HOURWISE_TEST_TURSO_TOKEN ? "set" : "NOT set"})`,
  );
}

function run(cmd, cmdArgs, opts = {}) {
  const res = spawnSync(cmd, cmdArgs, { stdio: "inherit", env, ...opts });
  if (res.error) return null;
  return res.status ?? 1;
}

const hasLocalCargo =
  process.env.HOURWISE_CARGO !== "docker" &&
  spawnSync("cargo", ["--version"], {
    stdio: "ignore",
    shell: process.platform === "win32",
  }).status === 0;

let status;
if (hasLocalCargo) {
  status = run("cargo", args, {
    cwd: backend,
    shell: process.platform === "win32",
  });
} else {
  console.error(
    "[cargo.mjs] cargo not found locally; running it in Docker (rust:1-bookworm)…",
  );
  const quoted = args.map((a) => `'${a.replace(/'/g, "'\\''")}'`).join(" ");
  // `-e NAME` (no value) copies NAME from the environment passed to `docker` above, so the
  // secret is never placed on the command line where other processes could read it.
  const forward = [...TEST_VARS, "HOURWISE_PROD_TURSO_URL"]
    .filter((n) => env[n])
    .flatMap((n) => ["-e", n]);
  // `cargo run` starts the API server: publish its port and listen on all interfaces inside
  // the container (backend/.env is still read; this overrides only BIND_ADDR). `--init` makes
  // Ctrl+C stop the server.
  let serve = [];
  if (args[0] === "run") {
    if (!existsSync(resolve(backend, ".env"))) {
      console.error(
        "[cargo.mjs] backend/.env is missing. Create it first:  copy backend\\.env.example backend\\.env",
      );
      process.exit(1);
    }
    const port = process.env.BACKEND_DEV_PORT || "8080";
    serve = ["--init", "-p", `${port}:8080`, "-e", "BIND_ADDR=0.0.0.0:8080"];
  }
  status = run("docker", [
    "run",
    "--rm",
    "-v",
    `${backend}:/app`,
    "-v",
    "hourwise-cargo:/usr/local/cargo/registry",
    "-v",
    "hourwise-target:/app/target",
    "-w",
    "/app",
    ...forward,
    ...serve,
    "rust:1-bookworm",
    "sh",
    "-c",
    `rustup component add rustfmt clippy >/dev/null 2>&1; cargo ${quoted}`,
  ]);
  if (status === null) {
    console.error(
      "[cargo.mjs] Neither cargo nor docker is available. Install Rust: https://rustup.rs",
    );
    status = 1;
  }
}
process.exit(status);
