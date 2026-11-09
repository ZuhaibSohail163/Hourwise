# Hourwise

Hourwise is a self-hosted team time tracker, loosely modelled on the Harvest timesheet.
Team members sign in with email and password and log hours and minutes per day against the
projects they belong to. Admins get their own interface where they create projects, manage
members, and see who worked on what, for how long and what they did.

- **Backend:** Rust ([axum](https://github.com/tokio-rs/axum)) REST API using the official
  [`libsql`](https://crates.io/crates/libsql) client
- **Frontend:** Next.js 16 (App Router, React 19, TypeScript, Tailwind CSS 4)
- **Database:** [Turso Cloud](https://turso.tech) (hosted libSQL/SQLite, free tier) in
  production, or a **local SQLite file** for development, with no account needed
- **Packaging:** Docker Compose. One command starts the whole stack.

---

## Features

### For everyone
- **Email + password sign-in.** Passwords are hashed with Argon2. Sessions are HttpOnly cookies
  with CSRF protection, and failed logins are rate limited.
- **Timesheet (Day view).** Harvest-style week strip with daily totals. Click *Track time* to log
  hours + minutes, choose a project and write notes about the work. Entries can be edited and deleted.
- **Timesheet (Week view).** A grid of projects × days with row, column and week totals.
- **Reports.** Switch between **Day / Week / Month** and move through periods. Each report shows:
  - total time, number of entries, days worked and the average per day worked
  - a daily hours chart (week/month)
  - a breakdown by project with percentages
  - a **work summary**: every entry and its notes, grouped day by day
  - CSV export
- **Profile.** Change your name, job title, password and profile picture. Your email is read-only
  (only an admin can change it).
- You can only log time to **active projects you're a member of**. This is enforced by the API.
  A day can't total more than 24 hours.

### For admins (separate interface at `/admin`)
- **Overview:** hours today / this week / this month, active projects, hours per member this week.
- **Projects:** create, edit, archive/unarchive and delete projects (name, client, description, colour).
  - See the **member count**, total hours, and **hours per member**.
  - Add and remove members. Removing someone keeps their logged history.
  - A *Time & work* tab with Day/Week/Month reports for the project, filterable by member,
    including every description of the work done.
- **Team:** add people (with an initial password, a role and the projects they join), and view
  **everything about each user**: profile, projects with hours per project, and their full time
  and work log by day/week/month.
  - Change a user's **email**, name, job title and role, activate or deactivate them, reset their password, or delete them.
  - Safety rails: you can't demote, deactivate or delete yourself.
- **Reports:** organisation-wide reports filterable by project and person, with breakdowns
  by project and by member, the full work log, and CSV export.

---

## Quick start (Docker, local database)

**Prerequisites:** [Docker Desktop](https://www.docker.com/products/docker-desktop/) (or Docker
Engine + Compose v2), Git and Node.js 20+ (Node is only needed for the helper scripts).

```bash
# 1. Clone
git clone <your-repo-url> hourwise
cd hourwise

# 2. Create your environment file
cp .env.example .env          # Windows PowerShell: Copy-Item .env.example .env
```

3. Edit `.env` and set at least:
   - `JWT_SECRET`: a random string of **32+ characters**. Generate one with `openssl rand -hex 32`
     (or in PowerShell: `-join ((48..57)+(97..102) | Get-Random -Count 64 | % {[char]$_})`).
   - `ADMIN_EMAIL` / `ADMIN_PASSWORD`: the first admin account.
   - Leave `TURSO_DATABASE_URL` / `TURSO_AUTH_TOKEN` **empty** to use a local database file.

```bash
# 4. Build and start everything (the first build takes a few minutes while Rust compiles)
docker compose up -d --build
```

5. Open **http://localhost:3000** and sign in with `ADMIN_EMAIL` / `ADMIN_PASSWORD`.

To use Turso Cloud instead, follow [Turso Cloud setup](#turso-cloud-setup), fill in the two
`TURSO_*` variables and run `docker compose up -d --build` again.

### First steps as an admin
1. Go to **Profile** and change the admin password.
2. **Admin → Projects → New project** to create your projects.
3. **Admin → Team → Add person** to create accounts for your team. Give each person a temporary
   password and pick the projects they should join.
4. Share the URL and credentials with each person. They sign in, change their password on their
   Profile page and start tracking time.

> The initial admin is created **only if the database has no admin yet**, so changing
> `ADMIN_PASSWORD` later has no effect. Use the Profile page instead.

### Everyday commands

| Task | Command |
| --- | --- |
| Start / rebuild | `docker compose up -d --build` |
| Stop | `docker compose down` (data is kept) |
| View logs | `docker compose logs -f backend` (or `frontend`) |
| Check DB connection + credentials | `docker compose exec backend hourwise-backend check-db` |
| Apply migrations (normally automatic) | `docker compose exec backend hourwise-backend migrate` |
| Data integrity report | `docker compose exec backend hourwise-backend verify` |
| **Back up (verified)** | `npm run backup` → `backups/hourwise-<timestamp>.sql` |
| Run all tests | `npm test` (Rust + frontend); `npm run test:e2e` (browser) |
| **Wipe local data** | `docker compose down -v` (**deletes** the local DB and uploads) |

Local data lives in Docker volumes: `dbdata` (the local database file, only when Turso isn't
configured) and `uploads` (profile pictures).

---

## Database

Hourwise stores everything in four tables (see [Data model](#data-model)). The backend talks to
the database through the official [`libsql`](https://crates.io/crates/libsql) Rust crate, which
supports two kinds of target with the same SQL and code:

| `TURSO_DATABASE_URL` | Where the data lives | Use for |
| --- | --- | --- |
| `libsql://<db>-<org>.turso.io` (+ `TURSO_AUTH_TOKEN`) | **Turso Cloud**, over HTTPS | production / shared environments |
| `file:/data/db/hourwise.db` (Docker default) or `file:./data/hourwise.db` | a local SQLite file | development, tests, offline use (free) |
| `http://127.0.0.1:8081` | a local `turso dev` / `sqld` server | optional: testing the network path offline |

**Why `libsql` and not SQLx?** SQLx's SQLite driver can only open local files, not a remote
Turso URL. `libsql` is Turso's production client for libSQL databases (remote over HTTPS, plus
local files). The new `turso` crate is mainly meant for local and embedded-sync use, and
`turso_serverless` targets the newer "Turso database" engine. Queries remain asynchronous and
fully parameterised (`?1, ?2…` placeholders with bound values; see `backend/src/db.rs`).

### Turso Cloud setup

You need a free Turso account. Hourwise only needs **one database**, which fits easily in the
free plan: at the time of writing it includes 100 databases, 5 GB of storage, 500 M rows read and
10 M rows written per month, and 1 day of point-in-time restore. Check
[turso.tech/pricing](https://turso.tech/pricing) for the current limits. A team of ten people logs
a few hundred entries a week, which is far below these limits.

1. **Create an account:** sign up at <https://app.turso.tech> (GitHub or email).
2. **Install the CLI** (optional; you can also use the web dashboard for steps 3–5):
   - macOS: `brew install tursodatabase/tap/turso`
   - Linux / WSL: `curl -sSfL https://get.tur.so/install.sh | bash`
   - Windows: use WSL, or use the web dashboard.

   Then run `turso auth login`.
3. **Choose a region close to where the backend will run** (lower latency per query). In the
   dashboard you pick it when creating the database. With the CLI, `turso db locations` lists
   them. Databases live in a *group* whose location you set when the group is created, for example
   `turso group create hourwise --location aws-eu-west-1`.
4. **Create the database** as a standard libSQL database (the default; don't use the
   `--tursodb` engine flag):
   ```bash
   turso db create hourwise            # add --group hourwise if you created a group above
   ```
5. **Get the URL and create an auth token:**
   ```bash
   turso db show hourwise --url        # libsql://hourwise-<your-org>.turso.io
   turso db tokens create hourwise     # long random string; treat it like a password
   ```
   For an expiring token use `turso db tokens create hourwise --expiration 90d`, and rotate it
   before it expires. In the dashboard: open the database → **Create Token**.
6. **Add them to `.env`** (never commit this file):
   ```env
   TURSO_DATABASE_URL=libsql://hourwise-<your-org>.turso.io
   TURSO_AUTH_TOKEN=<the token>
   ```
7. **Initialise the schema.** The backend applies all migrations on startup, and you can also
   run them explicitly first:
   ```bash
   docker compose build backend
   docker compose run --rm backend hourwise-backend check-db   # verifies URL + token
   docker compose run --rm backend hourwise-backend migrate
   ```
8. **Start Hourwise:** `docker compose up -d --build`. The backend log says
   `using Turso/libSQL server at libsql://…` (the token is never logged). Sign in with
   `ADMIN_EMAIL` / `ADMIN_PASSWORD` to create the first admin in the cloud database.

Switching between the local file and Turso is just a matter of setting or clearing the two
variables. The two databases are separate; use [backup and restore](#backups-and-recovery) to move
data between them.

### Migrations

- Schema files live in `backend/migrations/NNNN_name.sql`. They are compiled into the binary and
  registered in `backend/src/migrate.rs`.
- They are applied in order, each in its own transaction, on startup or with
  `hourwise-backend migrate`. Applied versions are recorded in `_hourwise_migrations` with a
  checksum, so an edited old migration is detected and the backend refuses to start.
- **Never edit an applied migration.** Add a new file and append it to `MIGRATIONS`.
- Before running a new migration against production, take a backup (`npm run backup`).

---

## Backups and recovery

Turso's point-in-time restore (1 day on the free plan) protects against accidents within the
last day. It is **not** a backup: it lives in the same account, and a mistake noticed after a day,
or a lost account or token, is not covered. Keep independent backups too.

**Logical backups (recommended, works on the free plan):**

```bash
npm run backup                 # backups/hourwise-YYYYMMDD-HHMMSS.sql + .sha256
npm run backup -- --keep 30    # same, and keep only the newest 30 local backups
```

`scripts/backup.mjs`:
1. runs `hourwise-backend backup -` inside the running backend container. This works for both
   Turso Cloud and the local file, and needs no Turso CLI.
2. **test-restores** the file into a throwaway database in a temporary container and runs the
   integrity checks. The real database is never written to.
3. writes a SHA-256 checksum.

The backup is a plain-text SQL file with one `INSERT` per row and a header holding the schema
version and row counts. `restore` only accepts `INSERT INTO <hourwise table>` statements, one per
line, so a tampered file can't run other SQL.

**Schedule it.** Run it daily, for example with cron (`0 2 * * * cd /srv/hourwise && npm run
backup -- --keep 30`) or Windows Task Scheduler. Copy the files **off the server**.

**Store backups securely.** They contain employee names, emails, timesheets and password hashes.
Encrypt them before uploading them anywhere, for example `age -r <public-key> file.sql > file.sql.age`
or `gpg -c file.sql`. Store them in a private bucket or drive with restricted access, and keep
at least one copy outside your Turso account.

**Alternatives using the Turso CLI:**
- `turso db shell hourwise .dump > dump.sql` (SQL dump)
- `turso db export hourwise --output-file hourwise.db` (SQLite file snapshot; the docs note it
  may lag the latest writes)

**Restore / disaster recovery:**
1. Create a **new, empty** database (`turso db create hourwise-restore`), or point
   `TURSO_DATABASE_URL` at a fresh local file.
2. Check the file: `sha256sum -c backups/<file>.sql.sha256`.
3. Restore into it: `docker compose run --rm -v "$PWD/backups:/b:ro" backend hourwise-backend restore /b/<file>.sql`.
   Restoring into a non-empty database is refused.
4. Check it: `docker compose run --rm backend hourwise-backend verify` (row counts, orphaned
   rows, entries outside 1–1440 minutes, days over 24 h, invalid dates, email uniqueness, an
   active admin, and SQLite `integrity_check` where available).
5. Switch `.env` to the restored database and `docker compose up -d`.

Profile pictures are files in the `uploads` volume, not in the database. Back them up separately,
for example `docker run --rm -v hourwise_uploads:/u -v "$PWD:/b" alpine tar czf /b/uploads.tgz -C /u .`.

**Integrity checks at any time:** `docker compose exec backend hourwise-backend verify`. The
exit code is non-zero if a problem is found, so it can be used in monitoring.

---

## Local development (without Docker for the app)

**Prerequisites:** Node.js 20+, and either Docker **or** [Rust](https://rustup.rs/) (stable) with
a C compiler (the local SQLite engine is compiled from C: build-essential on Linux, Xcode CLT on
macOS, MSVC Build Tools on Windows).

```bash
# 1. Backend config: uses a local database file by default (backend/data/hourwise.db)
cp backend/.env.example backend/.env    # Windows: copy backend\.env.example backend\.env
                                        # set JWT_SECRET; optionally point TURSO_* at Turso

# 2. Backend (from the repo root): http://localhost:8080, migrations run automatically
npm run dev:backend

# 3. Frontend (new terminal, from the repo root): http://localhost:3000, proxies /api to :8080
npm --prefix frontend install
npm run dev:frontend
```

`npm run dev:backend` runs `cargo run` with your local Rust toolchain if you have one. Otherwise
it runs it inside the `rust:1-bookworm` Docker image with port 8080 published, so **you don't need
Rust installed**. The first Docker build takes a few minutes; later runs are cached. Press Ctrl+C
to stop it. Use a different host port with `BACKEND_DEV_PORT=8081`.

> **`cargo: The term 'cargo' is not recognized`** (PowerShell) means Rust isn't installed. Either
> use `npm run dev:backend` as above, or install Rust with `winget install Rustlang.Rustup`
> (plus the "Desktop development with C++" workload from Visual Studio Build Tools), open a new
> terminal, then run `cd backend; cargo run`.

If the backend runs somewhere other than `http://localhost:8080`, start the frontend with
`BACKEND_URL=http://host:port npm run dev`. Every `npm run …` command at the repo root falls back
to running cargo in Docker when Rust isn't installed (`scripts/cargo.mjs`).

> If you also run the full stack with `docker compose up`, the containers use ports 8080 and 3000
> as well. Stop them first, or change `BACKEND_PORT` / `FRONTEND_PORT` in `.env`.

---

## Testing

Every test runs against **isolated, throwaway data**. Backend tests create a fresh SQLite file
per test in a temp directory, and the browser tests run against a separate Docker stack whose
database lives in memory. None of them read `.env` or touch your real database.

| Command (repo root) | What runs |
| --- | --- |
| `npm test` | all Rust tests + all frontend tests |
| `npm run test:unit` | Rust unit tests (validation, durations, dates, auth, rate limiting, backup format) |
| `npm run test:integration` | Rust database, API and security integration tests |
| `npm run test:backend` | all Rust tests (`cargo test`) |
| `npm run test:frontend` | Vitest + React Testing Library (`frontend/src/__tests__`) |
| `npm run test:e2e` | Playwright end-to-end tests on an isolated Docker stack (ports 3100/8180) |
| `npm run test:turso` | the real Turso Cloud test (see below) |
| `npm run test:all` | everything above except Turso |

- **Backend** (`backend/tests/`): `db.rs` covers the schema, migrations, constraints, foreign
  keys, transactions, concurrency, persistence, backup/restore and an unreachable database.
  `api.rs` covers every endpoint plus the business rules and reports. `security.rs` covers auth,
  CSRF, session expiry and revocation, rate limiting, SQL injection, permissions and outage
  handling. The shared harness is `tests/common/mod.rs`.
- **Frontend** (`frontend/src/__tests__/`): login, timesheet (day/week totals, delete), the entry
  form (validation, project selection), profile, the admin dashboard, users and projects pages,
  reports and CSV export, and the API client (CSRF header, 401 handling). The API is mocked
  here, so these tests don't prove database behaviour; the backend tests do that.
- **End-to-end** (`frontend/e2e/`): an admin creates an employee and a project, the employee
  logs time and sees the weekly total, the admin reviews it, the employee edits the entry,
  reports and CSV are checked, and the employee logs out. First run
  `npx --prefix frontend playwright install chromium`.
- **Turso Cloud** (`backend/tests/turso_cloud.rs`): ignored by default. It needs a **dedicated,
  empty** test database. It refuses to run if any Hourwise table already has rows, and it checks
  this read-only **before** applying migrations, so a wrong database is never altered. It deletes
  its rows afterwards, pass or fail.
  ```bash
  turso db create hourwise-test
  turso db show hourwise-test --url        # -> HOURWISE_TEST_TURSO_URL in the root .env
  turso db tokens create hourwise-test     # -> HOURWISE_TEST_TURSO_TOKEN in the root .env
  npm run test:turso
  ```
  `scripts/cargo.mjs` reads these two variables from the shell environment or, if they are not
  set there, from the root `.env` (so it works from WebStorm/VS Code on Windows without
  exporting anything). It forwards only these two into the Rust Docker container. The
  production `TURSO_DATABASE_URL` / `TURSO_AUTH_TOKEN` are never passed to tests. The runner
  and the test both refuse to start if the test URL equals `TURSO_DATABASE_URL`. Only the test
  host is printed, never the token.

  The test database is **not** part of `docker-compose.yml`. The app containers use only
  `TURSO_DATABASE_URL` / `TURSO_AUTH_TOKEN` (your production or development database).

CI (`.github/workflows/ci.yml`) runs formatting, lints, type checks, all Rust and frontend tests,
both builds and the Playwright suite on every push and pull request.

## Linting & formatting

| Tool | Scope | Config |
| --- | --- | --- |
| **rustfmt** | Rust formatting | `backend/rustfmt.toml` |
| **clippy** | Rust lints, with warnings treated as errors | `[lints]` in `backend/Cargo.toml` |
| **Prettier** (+ Tailwind class sorting) | TS/TSX/CSS/JSON formatting | `frontend/.prettierrc.json` |
| **ESLint** (Next.js core-web-vitals + TypeScript + React hooks rules) | TS/TSX lints | `frontend/eslint.config.mjs` |
| **TypeScript** | type checking | `frontend/tsconfig.json` |

```bash
npm run format          # auto-format backend + frontend
npm run lint            # clippy + eslint, fail on any warning
npm run lint:fix        # apply automatic lint fixes
npm run check           # format check + lints + type check
```

If Rust isn't installed locally, the backend commands automatically run `cargo` inside the
`rust:1-bookworm` Docker image (via `scripts/cargo.mjs`). Set `HOURWISE_CARGO=docker` to force that.
`.editorconfig` and `.gitattributes` keep indentation and LF line endings consistent.

---

## Configuration

### Root `.env` (read by Docker Compose)

| Variable | Default | Description |
| --- | --- | --- |
| `TURSO_DATABASE_URL` | local file in the `dbdata` volume | `libsql://…turso.io` for Turso Cloud. |
| `TURSO_AUTH_TOKEN` | – | Required for Turso Cloud. Secret. |
| `HOURWISE_TEST_TURSO_URL` / `HOURWISE_TEST_TURSO_TOKEN` | – | Only for `npm run test:turso`: a separate, empty test database. Not used by the containers. |
| `JWT_SECRET` | *(required)* | Signs session cookies. At least 32 characters. Changing it signs everyone out. |
| `JWT_TTL_HOURS` | `168` | Session lifetime (cookie `Max-Age` and token expiry). |
| `COOKIE_SECURE` | `true` | Adds `Secure` to cookies. Browsers accept this on `http://localhost`; set `false` only to test over plain HTTP on another host. |
| `LOGIN_MAX_ATTEMPTS` / `LOGIN_LOCKOUT_SECS` | `5` / `900` | Failed logins per email before a temporary lockout. |
| `ADMIN_EMAIL` / `ADMIN_PASSWORD` / `ADMIN_NAME` | – | Initial admin, created on startup only when no admin exists. |
| `BACKEND_PORT` / `FRONTEND_PORT` | `8080` / `3000` | Host ports. |

### Backend environment (`backend/.env` when running with `cargo run`)

Same variables as above, plus:

| Variable | Default | Description |
| --- | --- | --- |
| `TURSO_DATABASE_URL` | *(required)* | e.g. `file:./data/hourwise.db` or `libsql://…`. |
| `BIND_ADDR` | `0.0.0.0:8080` | Address the API listens on. |
| `UPLOAD_DIR` | `./uploads` | Where profile pictures are stored. |
| `CORS_ORIGIN` | *(unset)* | Only needed if a browser calls the API directly instead of through the Next.js proxy. |
| `RUST_LOG` | `hourwise_backend=info,tower_http=info` | Log level. |

---

## Architecture

```
Browser ──► Next.js (port 3000) ──/api/*, /uploads/*──► Rust API (port 8080) ──HTTPS──► Turso Cloud
            pages + proxy                                axum + libsql        └─(or)──► local SQLite file
```

- The browser only talks to the Next.js server, which proxies `/api/*` and `/uploads/*` to the
  Rust backend (`rewrites` in `frontend/next.config.ts`). Cookies are therefore first-party and
  there's no CORS setup. `BACKEND_URL` is resolved **at build time**; Docker passes it as a build arg.
- The backend opens a connection per request (an HTTPS client for Turso, or a SQLite connection
  with `foreign_keys=ON` and a busy timeout for local files). Writes that check a rule and then
  write (the 24 h cap) run in `BEGIN IMMEDIATE` transactions so concurrent requests can't race.
  Cascading deletes are done explicitly in a transaction, so they don't depend on the
  per-connection `PRAGMA foreign_keys` setting over HTTPS.
- At startup the backend retries the database connection for about a minute, applies pending
  migrations and seeds the first admin. Requests made while the database is unreachable get a
  `503` with a generic message, and details go only to the log.

### Authentication and security

- `POST /api/auth/login` verifies the Argon2 hash and sets two cookies:
  - `hw_session`: a signed JWT; `HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=<ttl>`
  - `hw_csrf`: a random token bound to that session, readable by the page
- Every non-GET request must send `X-CSRF-Token` matching the session's token, otherwise `403`.
  Together with `SameSite=Strict` and JSON-only bodies this blocks cross-site request forgery.
- The user row is re-read on every request: role changes and deactivation apply immediately.
  `users.session_epoch` is part of each session. Changing your password, an admin password reset
  or deactivation bumps it, which signs out every other session (your current browser gets a
  fresh cookie after a self-service password change).
- Failed logins are limited per email: 5 per 15 minutes by default, then `429` with
  `Retry-After`. Unknown emails take the same time as wrong passwords, so the timing doesn't
  reveal which emails have accounts.
- All SQL is parameterised. Uploads are size-limited and magic-byte checked. CSV exports
  neutralise spreadsheet formulas.
- `POST /api/auth/logout` expires the cookies.

### Data model

| Table | Purpose |
| --- | --- |
| `users` | id (UUID text), email (unique, lower-cased), name, Argon2 hash, role (`admin`/`member`), job title, avatar, active flag, session epoch |
| `projects` | name, client, description, colour, archived flag |
| `project_members` | which users may log time to which projects (composite primary key) |
| `time_entries` | user, project, date (`YYYY-MM-DD`), minutes (1–1440), description |

Tables are SQLite `STRICT` tables with `CHECK` constraints for every format: UUID length,
booleans 0/1, the date pattern, the minutes range, the role and lower-case emails. Timestamps are
RFC 3339 UTC strings with millisecond precision. Durations are integer minutes, so totals are exact.

### REST API

All endpoints are under `/api`. Everything except `/health`, `/auth/login` and `/auth/logout`
requires the session cookie, and non-GET requests also require `X-CSRF-Token`.

| Method & path | Who | Description |
| --- | --- | --- |
| `POST /auth/login` | public | `{email, password}` → `{user}` + session cookies |
| `POST /auth/logout` | public | Clears the session cookies |
| `GET /me` | user | Current user |
| `PATCH /me` | user | `{name?, job_title?}` (email can't be changed here) |
| `PUT /me/password` | user | `{current_password, new_password}`; signs out other sessions |
| `POST /me/avatar` | user | multipart `file` (PNG/JPEG/WEBP/GIF, ≤ 2 MB) |
| `DELETE /me/avatar` | user | Remove profile picture |
| `GET /projects` | user | Projects the user is a member of |
| `GET /time-entries?from&to&project_id` | user | Own entries (dates are `YYYY-MM-DD`) |
| `POST /time-entries` | user | `{project_id, entry_date, minutes, description}` |
| `PATCH /time-entries/{id}` / `DELETE …` | user | Edit / delete own entry |
| `GET /admin/stats` | admin | Dashboard numbers |
| `GET, POST /admin/users` | admin | List (with hours and project counts) / create `{email, name, password, role?, job_title?, project_ids?}` |
| `GET, PATCH, DELETE /admin/users/{id}` | admin | Detail with projects / update `{email?, name?, role?, job_title?, is_active?, password?}` / delete |
| `GET, POST /admin/projects` | admin | List (member count, total hours) / create |
| `GET, PATCH, DELETE /admin/projects/{id}` | admin | Detail with members and hours / update (incl. `is_archived`) / delete |
| `POST /admin/projects/{id}/members` | admin | `{user_ids: [...]}` |
| `DELETE /admin/projects/{id}/members/{user_id}` | admin | Remove a member (keeps their history) |
| `GET /admin/time-entries?from&to&project_id&user_id` | admin | All entries, filterable |

Errors are returned as `{"error": "message"}` with the matching HTTP status.

---

## Project structure

```
.
├── docker-compose.yml        # backend + frontend
├── docker-compose.e2e.yml    # isolated stack for the Playwright tests
├── .env.example              # copy to .env
├── package.json              # repo-level lint/format/test/backup scripts (no dependencies)
├── scripts/
│   ├── cargo.mjs             # runs cargo locally, or in Docker if Rust isn't installed
│   ├── backup.mjs            # verified logical backups
│   └── e2e.mjs               # starts the isolated stack and runs Playwright
├── .github/workflows/ci.yml
├── backend/                  # Rust API
│   ├── migrations/           # SQLite/libSQL migrations (embedded in the binary)
│   ├── src/
│   │   ├── main.rs           # CLI: serve | migrate | check-db | verify | backup | restore
│   │   ├── lib.rs            # app + state construction (shared with tests)
│   │   ├── db.rs             # libsql connection, value binding, row mapping
│   │   ├── migrate.rs        # migration runner
│   │   ├── backup.rs         # export / import / verify
│   │   ├── config.rs         # env configuration
│   │   ├── auth.rs           # Argon2, session cookies, CSRF, rate limiting, extractors
│   │   ├── error.rs          # AppError → JSON responses
│   │   ├── models.rs         # row / response types
│   │   └── routes/           # auth, me, projects, entries, admin
│   └── tests/                # db, api, security, turso_cloud integration tests
└── frontend/                 # Next.js app
    ├── e2e/                  # Playwright specs
    ├── vitest.config.mts, playwright.config.ts
    └── src/
        ├── __tests__/        # Vitest + React Testing Library
        ├── app/              # login, (app)/ member interface, admin/ interface
        ├── components/       # UI kit, EntryModal, SummaryReport, PeriodNav, ...
        └── lib/              # api client, auth context, csv, hooks, time helpers, types
```

---

## Production deployment notes

- **HTTPS is required:** session cookies are `Secure`. Put a reverse proxy (Caddy, Nginx or
  Traefik) or a platform's TLS in front of port 3000. Only the frontend needs to be public;
  remove the `backend` `ports:` mapping so the API is only reachable through the proxy.
- **Secrets:** use a strong unique `JWT_SECRET`, keep `TURSO_AUTH_TOKEN` in the host's secret
  store or `.env` with restricted permissions, and never commit either. Use a token scoped to the
  Hourwise database and rotate it periodically (`turso db tokens create … --expiration`).
- **Region:** run the backend in, or near, the same region as the Turso database. Each request
  makes a few round trips to the database.
- **Single backend instance:** the login rate limiter is in memory. That's fine for one
  instance; with several replicas each one has its own limit. Use sticky sessions or add a
  shared limiter.
- **Uploads** live on the backend's disk (`uploads` volume). On platforms with ephemeral disks,
  mount a persistent volume or avatars are lost on redeploy. The database is unaffected.
- **Backups:** schedule `npm run backup`, store the files encrypted and off-site, and test a
  restore now and then.
- Weeks start on Monday. Dates are calendar dates without a time zone, so an entry logged for
  "Tuesday" stays on Tuesday for everyone. The API accepts dates up to one day ahead of UTC, so
  people east of UTC can log "today".

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| `TURSO_DATABASE_URL must be set` (cargo run) | Copy `backend/.env.example` to `backend/.env`. |
| `TURSO_AUTH_TOKEN must be set …` | A `libsql://` URL needs a token: `turso db tokens create <db>`. |
| `database not reachable` / `401` / `Unauthorized` in backend logs | Token expired or for another database. Create a new one. Check with `hourwise-backend check-db`. |
| `error trying to connect` / DNS / timeout | Check the URL (`turso db show <db> --url`), internet access and proxy or firewall rules from the container. |
| `503 The database is temporarily unavailable` in the app | The backend can't reach the database. Check the logs, then `check-db`. |
| `migration N was modified after it was applied` | An applied migration file was edited. Revert it and add a new migration instead. |
| `database is locked` with a local file | Another process holds the file (for example a SQLite GUI). Close it. |
| Logged out immediately / login loops over plain HTTP on a LAN IP | `Secure` cookies need HTTPS or `localhost`. Use HTTPS, or set `COOKIE_SECURE=false` for local testing only. |
| `403 You do not have permission` on every save | The CSRF cookie is missing, often after clearing cookies. Sign out and in again. |
| `429 Too many failed attempts` | Wait `LOGIN_LOCKOUT_SECS` (default 15 min), or restart the backend to clear the in-memory limiter. |
| `Set JWT_SECRET in .env` when running compose | Create `.env` from `.env.example` and set `JWT_SECRET`. |
| Can't log in as admin after changing `ADMIN_PASSWORD` | The seed only runs when no admin exists. Ask another admin to reset it, or restore from a backup. |
| "Port is already allocated" | Change `BACKEND_PORT` or `FRONTEND_PORT` in `.env`. |
| Frontend shows network errors in dev | Make sure the backend is running on `http://localhost:8080` or set `BACKEND_URL`. |
| A member sees no projects in "Track time" | An admin needs to add them to an active project. |
