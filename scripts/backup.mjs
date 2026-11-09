#!/usr/bin/env node
// Creates a logical backup of the Hourwise database and proves it can be restored.
//
//   npm run backup                 # -> backups/hourwise-YYYYMMDD-HHMMSS.sql (+ .sha256)
//   npm run backup -- --keep 30    # also delete local backups older than the newest 30
//
// It uses the running `backend` container (docker compose), so it backs up whatever database
// the app is configured for: Turso Cloud when TURSO_DATABASE_URL is set, otherwise the local file.
// Steps:
//   1. `hourwise-backend backup -` streams the backup out of the container.
//   2. The file is restored into a THROWAWAY database inside a temporary container and checked
//      with `hourwise-backend verify` (row counts + integrity checks). The real database is
//      never written to.
//   3. A SHA-256 checksum is written next to the file.
// Store the `backups/` folder somewhere safe and private (it contains employee data and
// password hashes); see README -> "Backups and recovery".

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const dir = join(root, "backups");
const keepIdx = process.argv.indexOf("--keep");
const keep = keepIdx > 0 ? Number(process.argv[keepIdx + 1]) : 0;

function docker(args, opts = {}) {
  return spawnSync("docker", args, { cwd: root, maxBuffer: 1024 * 1024 * 1024, ...opts });
}

mkdirSync(dir, { recursive: true });
const stamp = new Date().toISOString().replace(/[-:]/g, "").replace("T", "-").slice(0, 15);
const name = `hourwise-${stamp}.sql`;
const file = join(dir, name);

console.error("[backup] exporting…");
const exp = docker(["compose", "exec", "-T", "backend", "hourwise-backend", "backup", "-"]);
if (exp.status !== 0) {
  process.stderr.write(exp.stderr);
  console.error("[backup] export failed. Is the stack running? (docker compose up -d)");
  process.exit(1);
}
writeFileSync(file, exp.stdout, { mode: 0o600 });

console.error("[backup] test-restoring into a throwaway database…");
const verify = docker(
  [
    "compose",
    "run",
    "--rm",
    "--no-deps",
    "-T",
    "-u",
    "0",
    "-e",
    "TURSO_DATABASE_URL=file:/tmp/restore-check.db",
    "-e",
    "TURSO_AUTH_TOKEN=",
    "-v",
    `${dir}:/backups:ro`,
    "--entrypoint",
    "sh",
    "backend",
    "-c",
    `hourwise-backend restore /backups/${name} && hourwise-backend verify`,
  ],
  { stdio: ["ignore", "inherit", "inherit"] },
);
if (verify.status !== 0) {
  console.error(`[backup] RESTORE CHECK FAILED - keep the file for investigation: ${file}`);
  process.exit(1);
}

const sum = createHash("sha256").update(exp.stdout).digest("hex");
writeFileSync(`${file}.sha256`, `${sum}  ${name}\n`);
console.error(`[backup] OK: ${file} (${exp.stdout.length} bytes, sha256 ${sum.slice(0, 12)}…)`);

if (keep > 0) {
  const old = readdirSync(dir)
    .filter((f) => /^hourwise-\d{8}-\d{6}\.sql$/.test(f))
    .sort((a, b) => statSync(join(dir, b)).mtimeMs - statSync(join(dir, a)).mtimeMs)
    .slice(keep);
  for (const f of old) {
    rmSync(join(dir, f));
    rmSync(join(dir, `${f}.sha256`), { force: true });
  }
  if (old.length) console.error(`[backup] removed ${old.length} old backup(s)`);
}
