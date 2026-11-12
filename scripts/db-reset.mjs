#!/usr/bin/env node
// Resets the LOCAL DEVELOPMENT PostgreSQL database (Docker volume `hourwise_postgres_data`).
//
//   npm run db:reset             # shows what would be deleted, deletes nothing
//   npm run db:reset -- --yes    # deletes the local dev database and starts an empty one
//
// It only ever removes that one Docker volume of the `hourwise` compose project. It never touches
// the legacy `hourwise_pgdata` volume, the uploads volume, the test servers, or any remote or
// production database. Take a backup first if the data matters: `npm run backup`.

import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const VOLUME = "hourwise_postgres_data";

function docker(args, inherit = true) {
  const res = spawnSync("docker", args, { cwd: root, stdio: inherit ? "inherit" : "pipe" });
  return { status: res.error ? 1 : (res.status ?? 1), out: res.stdout?.toString() ?? "" };
}

const exists = docker(["volume", "ls", "-q", "--filter", `name=^${VOLUME}$`], false).out.trim();
if (exists !== VOLUME) {
  console.error(`[db:reset] volume ${VOLUME} does not exist; nothing to reset.`);
  process.exit(0);
}
// Refuse unless the volume really belongs to this compose project's `db` service.
const labels = docker(["volume", "inspect", "-f", "{{json .Labels}}", VOLUME], false).out;
if (!labels.includes('"com.docker.compose.project":"hourwise"')) {
  console.error(`[db:reset] refusing: ${VOLUME} is not managed by the hourwise compose project.`);
  process.exit(1);
}
if (!process.argv.includes("--yes")) {
  console.error(
    `[db:reset] This would permanently delete the local development database (volume ${VOLUME}).\n` +
      "           Re-run with --yes to do it:  npm run db:reset -- --yes",
  );
  process.exit(1);
}

docker(["compose", "stop", "backend", "db"]);
docker(["compose", "rm", "-f", "db"]);
if (docker(["volume", "rm", VOLUME]).status !== 0) process.exit(1);
if (docker(["compose", "up", "-d", "--wait", "db"]).status !== 0) process.exit(1);
console.error(
  "[db:reset] done: empty database running. Migrations and the admin seed run when the backend starts.",
);
