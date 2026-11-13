#!/usr/bin/env node
// Backs up the LOCAL Docker Compose PostgreSQL database and proves the backup restores.
//
//   npm run backup                 # -> backups/hourwise-YYYYMMDD-HHMMSS.dump (+ .sha256)
//   npm run backup -- --keep 30    # also delete local backups older than the newest 30
//
// Steps:
//   1. `pg_dump --format=custom` runs inside the `db` container (read-only for the database).
//   2. The dump is restored into a THROWAWAY PostgreSQL container (tmpfs, removed afterwards)
//      and the row counts of every table are compared with the source. The real database is
//      never written to.
//   3. A SHA-256 checksum is written next to the file.
// Production backups on Dokploy use Dokploy's scheduled database backups instead (README ->
// "Backups and recovery"). Backups contain employee data and password hashes: keep them private.

import { spawnSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const dir = join(root, "backups");
const keepIdx = process.argv.indexOf("--keep");
const keep = keepIdx > 0 ? Number(process.argv[keepIdx + 1]) : 0;
const TABLES = ["users", "projects", "project_members", "time_entries"];

function readEnv() {
  const file = join(root, ".env");
  const out = {};
  if (!existsSync(file)) return out;
  for (const line of readFileSync(file, "utf8").split(/\r?\n/)) {
    const m = line.match(/^\s*([A-Z_]+)\s*=\s*(.*?)\s*$/);
    if (m) out[m[1]] = m[2].replace(/^["']|["']$/g, "");
  }
  return out;
}
const dotenv = readEnv();
const user = process.env.POSTGRES_USER || dotenv.POSTGRES_USER || "hourwise";
const database = process.env.POSTGRES_DB || dotenv.POSTGRES_DB || "hourwise";

function docker(args, opts = {}) {
  return spawnSync("docker", args, { cwd: root, maxBuffer: 2 * 1024 * 1024 * 1024, ...opts });
}
function fail(msg, res) {
  if (res?.stderr?.length) process.stderr.write(res.stderr);
  console.error(`[backup] ${msg}`);
  process.exit(1);
}

const countSql = TABLES.map((t) => `SELECT '${t}', COUNT(*) FROM ${t}`).join(" UNION ALL ");
function counts(execPrefix, db, dbUser) {
  const res = docker([...execPrefix, "psql", "-U", dbUser, "-d", db, "-At", "-F", "=", "-c", countSql]);
  if (res.status !== 0) return null;
  return res.stdout.toString().trim().split(/\r?\n/).sort().join(" ");
}

mkdirSync(dir, { recursive: true });
const stamp = new Date().toISOString().replace(/[-:]/g, "").replace("T", "-").slice(0, 15);
const name = `hourwise-${stamp}.dump`;
const file = join(dir, name);

console.error(`[backup] dumping database "${database}" from the db container…`);
const dump = docker([
  "compose", "exec", "-T", "db",
  "pg_dump", "-U", user, "-d", database, "--format=custom", "--no-owner", "--no-privileges",
]);
if (dump.status !== 0 || !dump.stdout.length) {
  fail("pg_dump failed. Is the stack running? (docker compose up -d db)", dump);
}
const sourceCounts = counts(["compose", "exec", "-T", "db"], database, user);
if (!sourceCounts) fail("could not count rows in the source database");
writeFileSync(file, dump.stdout, { mode: 0o600 });

console.error("[backup] test-restoring into a throwaway PostgreSQL container…");
const check = `hourwise-restore-check-${randomBytes(4).toString("hex")}`;
const started = docker([
  "run", "-d", "--rm", "--name", check,
  "-e", "POSTGRES_PASSWORD=restore-check-only", "-e", "POSTGRES_DB=restore_check",
  "--tmpfs", "/var/lib/postgresql",
  "postgres:18-alpine",
]);
if (started.status !== 0) fail("could not start the throwaway restore container", started);
let ok = false;
try {
  for (let i = 0; i < 60; i++) {
    if (docker(["exec", check, "pg_isready", "-U", "postgres", "-d", "restore_check"]).status === 0) break;
    spawnSync(process.execPath, ["-e", "setTimeout(()=>{},1000)"]);
  }
  const restore = docker(
    ["exec", "-i", check, "pg_restore", "-U", "postgres", "-d", "restore_check",
      "--no-owner", "--no-privileges", "--exit-on-error"],
    { input: dump.stdout },
  );
  if (restore.status !== 0) fail(`RESTORE FAILED - keep the file for investigation: ${file}`, restore);
  const restoredCounts = counts(["exec", check], "restore_check", "postgres");
  if (restoredCounts !== sourceCounts) {
    fail(`ROW COUNTS DIFFER after restore (source: ${sourceCounts}; restored: ${restoredCounts})`);
  }
  console.error(`[backup] restore check OK: ${restoredCounts}`);
  ok = true;
} finally {
  docker(["stop", check]);
}
if (!ok) process.exit(1);

const sum = createHash("sha256").update(dump.stdout).digest("hex");
writeFileSync(`${file}.sha256`, `${sum}  ${name}\n`);
console.error(`[backup] OK: ${file} (${dump.stdout.length} bytes, sha256 ${sum.slice(0, 12)}…)`);

if (keep > 0) {
  const old = readdirSync(dir)
    .filter((f) => /^hourwise-\d{8}-\d{6}\.dump$/.test(f))
    .sort((a, b) => statSync(join(dir, b)).mtimeMs - statSync(join(dir, a)).mtimeMs)
    .slice(keep);
  for (const f of old) {
    rmSync(join(dir, f));
    rmSync(join(dir, `${f}.sha256`), { force: true });
  }
  if (old.length) console.error(`[backup] removed ${old.length} old backup(s)`);
}
