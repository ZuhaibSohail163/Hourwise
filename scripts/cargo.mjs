#!/usr/bin/env node
// Runs `cargo <args>` in ./backend.
// Uses a local Rust toolchain when available, otherwise falls back to the official
// rust Docker image (so contributors without Rust installed can still build, lint and test).
//
//   node scripts/cargo.mjs fmt --check
//   node scripts/cargo.mjs clippy --all-targets -- -D warnings
//   HOURWISE_CARGO=docker node scripts/cargo.mjs fmt   # force Docker
//
// Test database safety (see backend/tests/common/mod.rs, which repeats every check):
// - The tests connect only to HOURWISE_TEST_DATABASE_URL (shell environment, or the root .env).
//   It is never derived from DATABASE_URL.
// - Every DATABASE_URL found in the shell, the root .env and backend/.env (plus the Docker
//   Compose development database) is "protected". The tests refuse to run if the test server is
//   the same server as any of them, however the URL is written. Protected destinations are
//   forwarded to the tests as HOURWISE_PROTECTED_DATABASES (host:port/db, no credentials).
// - Passwords are never printed or put on a command line.

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const backend = resolve(root, "backend");
const args = process.argv.slice(2);

const TEST_URL = "HOURWISE_TEST_DATABASE_URL";
const PROTECTED = "HOURWISE_PROTECTED_DATABASES";

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

const LOCAL_HOSTS = new Set([
  "",
  "localhost",
  "127.0.0.1",
  "::1",
  "0:0:0:0:0:0:0:1",
  "0.0.0.0",
  "host.docker.internal",
  "host-gateway",
]);

/** Mirrors `db::destination` in Rust: host (normalised), port, database. No credentials. */
function destination(url) {
  try {
    const u = new URL(String(url).trim());
    if (u.protocol !== "postgres:" && u.protocol !== "postgresql:") return null;
    let host = decodeURIComponent(u.hostname)
      .replace(/^\[|\]$/g, "")
      .replace(/\.$/, "")
      .toLowerCase();
    if (LOCAL_HOSTS.has(host) || host.startsWith("127.")) host = "localhost";
    const port = u.port ? Number(u.port) : 5432;
    const user = decodeURIComponent(u.username);
    const db = decodeURIComponent(u.pathname.replace(/^\//, "")) || user;
    return { host, port, db };
  } catch {
    return null;
  }
}

const rootEnv = readDotEnv(resolve(root, ".env"));
const backendEnv = readDotEnv(resolve(backend, ".env"));
const env = { ...process.env };
if (!env[TEST_URL] && rootEnv[TEST_URL]) env[TEST_URL] = rootEnv[TEST_URL];

// Everything that could be a real database.
const protectedUrls = [process.env.DATABASE_URL, rootEnv.DATABASE_URL, backendEnv.DATABASE_URL];
// The Docker Compose development database (published on the host).
protectedUrls.push(
  `postgres://x@localhost:${rootEnv.POSTGRES_PORT || 5432}/${rootEnv.POSTGRES_DB || "hourwise"}`,
);
const protectedDests = protectedUrls.filter(Boolean).map(destination).filter(Boolean);
env[PROTECTED] = protectedDests.map((d) => `${d.host}:${d.port}/${d.db}`).join(",");

const testDest = env[TEST_URL] ? destination(env[TEST_URL]) : null;
if (env[TEST_URL] && !testDest) {
  console.error(`[cargo.mjs] ${TEST_URL} is not a postgres:// URL.`);
  process.exit(1);
}
if (testDest) {
  const clash = protectedDests.find((p) => p.host === testDest.host && p.port === testDest.port);
  if (clash) {
    console.error(
      `[cargo.mjs] Refusing to run: ${TEST_URL} (${testDest.host}:${testDest.port}) is the same ` +
        `server as a protected database (${clash.host}:${clash.port}/${clash.db}).\n` +
        "            Use the disposable server from docker-compose.test.yml (npm run test:backend).",
    );
    process.exit(1);
  }
  if (args[0] === "test") {
    console.error(
      `[cargo.mjs] test database server: ${testDest.host}:${testDest.port} (credentials hidden)`,
    );
  }
}

function run(cmd, cmdArgs, opts = {}) {
  const res = spawnSync(cmd, cmdArgs, { stdio: "inherit", env, ...opts });
  if (res.error) return null;
  return res.status ?? 1;
}

/** Inside the cargo container, "localhost" is the container itself: point at the host instead. */
function forContainer(url) {
  const d = destination(url);
  if (!d || d.host !== "localhost") return url;
  const u = new URL(url.trim());
  u.hostname = "host.docker.internal";
  return u.toString();
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
  console.error("[cargo.mjs] cargo not found locally; running it in Docker (rust:1-bookworm)…");
  const quoted = args.map((a) => `'${a.replace(/'/g, "'\\''")}'`).join(" ");
  if (env[TEST_URL]) env[TEST_URL] = forContainer(env[TEST_URL]);
  // `-e NAME` (no value) copies NAME from the environment passed to `docker`, so secrets are
  // never placed on the command line where other processes could read them.
  const forward = [TEST_URL, PROTECTED].filter((n) => env[n]).flatMap((n) => ["-e", n]);
  // `cargo run` starts the API server: publish its port, listen on all interfaces inside the
  // container, and reach a database on the host through host.docker.internal. backend/.env is
  // still read by the server; only BIND_ADDR (and DATABASE_URL's host) are overridden.
  // `--init` makes Ctrl+C stop the server.
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
    const dbUrl = process.env.DATABASE_URL || backendEnv.DATABASE_URL;
    if (dbUrl) {
      env.DATABASE_URL = forContainer(dbUrl);
      serve.push("-e", "DATABASE_URL");
    }
  }
  status = run("docker", [
    "run",
    "--rm",
    "--add-host=host.docker.internal:host-gateway",
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
