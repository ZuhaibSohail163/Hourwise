#!/usr/bin/env node
// Runs the Rust test suite against a disposable PostgreSQL server.
//
//   npm run test:backend                         # all Rust tests
//   npm run test:backend -- --test api           # one integration test file
//   HOURWISE_TEST_KEEP_DB=1 npm run test:backend # leave the test server running afterwards
//
// Unless HOURWISE_TEST_DATABASE_URL is already set, it starts the throwaway server from
// docker-compose.test.yml (tmpfs, port 127.0.0.1:55432, marked `hourwise.disposable=on`), runs
// `cargo test` through scripts/cargo.mjs, then removes the server again. The development
// database (docker-compose.yml) and any production database are never touched: see the safety
// rules in backend/tests/common/mod.rs.

import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const compose = ["compose", "-f", "docker-compose.test.yml", "-p", "hourwise-test"];
// Throwaway credentials of the local test container (see docker-compose.test.yml).
const DEFAULT_URL = "postgres://hourwise_test:hourwise_test@localhost:55432/postgres";

function run(cmd, args, env = process.env) {
  const res = spawnSync(cmd, args, { cwd: root, stdio: "inherit", env });
  return res.error ? 1 : (res.status ?? 1);
}

const env = { ...process.env };
let started = false;
if (!env.HOURWISE_TEST_DATABASE_URL) {
  console.error("[test-backend] starting the disposable PostgreSQL test server…");
  if (run("docker", [...compose, "up", "-d", "--wait"]) !== 0) {
    console.error("[test-backend] could not start docker-compose.test.yml. Is Docker running?");
    process.exit(1);
  }
  started = true;
  env.HOURWISE_TEST_DATABASE_URL = DEFAULT_URL;
}

let status = 1;
try {
  status = run(process.execPath, ["scripts/cargo.mjs", "test", ...process.argv.slice(2)], env);
} finally {
  if (started && !process.env.HOURWISE_TEST_KEEP_DB) {
    run("docker", [...compose, "down", "-v", "--remove-orphans"]);
  }
}
process.exit(status);
