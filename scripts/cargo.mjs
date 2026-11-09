#!/usr/bin/env node
// Runs `cargo <args>` in ./backend.
// Uses a local Rust toolchain when available, otherwise falls back to the official
// rust Docker image (so contributors without Rust installed can still lint/format).
//
//   node scripts/cargo.mjs fmt --check
//   node scripts/cargo.mjs clippy --all-targets -- -D warnings
//   HOURWISE_CARGO=docker node scripts/cargo.mjs fmt   # force Docker

import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const backend = resolve(dirname(fileURLToPath(import.meta.url)), "..", "backend");
const args = process.argv.slice(2);

function run(cmd, cmdArgs, opts = {}) {
  const res = spawnSync(cmd, cmdArgs, { stdio: "inherit", ...opts });
  if (res.error) return null;
  return res.status ?? 1;
}

const hasLocalCargo =
  process.env.HOURWISE_CARGO !== "docker" &&
  spawnSync("cargo", ["--version"], { stdio: "ignore", shell: process.platform === "win32" })
    .status === 0;

let status;
if (hasLocalCargo) {
  status = run("cargo", args, { cwd: backend, shell: process.platform === "win32" });
} else {
  console.error("[cargo.mjs] cargo not found locally; running it in Docker (rust:1-bookworm)…");
  const quoted = args.map((a) => `'${a.replace(/'/g, "'\\''")}'`).join(" ");
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
    "rust:1-bookworm",
    "sh",
    "-c",
    `rustup component add rustfmt clippy >/dev/null 2>&1; cargo ${quoted}`,
  ]);
  if (status === null) {
    console.error("[cargo.mjs] Neither cargo nor docker is available. Install Rust: https://rustup.rs");
    status = 1;
  }
}
process.exit(status);
