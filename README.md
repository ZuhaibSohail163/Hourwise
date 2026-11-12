# Hourwise

Hourwise is a self-hosted team time tracker, loosely modelled on the Harvest timesheet.
Team members sign in with email and password and log hours and minutes per day against the
projects they belong to. Admins get their own interface where they create projects, manage
members, and see who worked on what, for how long and what they did.

- **Backend:** Rust ([axum](https://github.com/tokio-rs/axum)) REST API with
  [SQLx](https://github.com/launchbadge/sqlx) and a PostgreSQL connection pool
- **Frontend:** Next.js 16 (App Router, React 19, TypeScript, Tailwind CSS 4)
- **Database:** PostgreSQL 14+ (17 in Docker)
- **Packaging:** Docker images for both apps. Docker Compose for local development. Production
  deployment with [Dokploy](https://dokploy.com): see **[docs/DEPLOYMENT.md](docs/DEPLOYMENT.md)**.

There are no user-count limits anywhere in the code.

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


---

## Architecture

```
Browser ──HTTPS──▶ Next.js frontend (:3000) ──/api, /uploads──▶ Rust API (:8080) ──▶ PostgreSQL (:5432)
                   the only public service       private network            private network
```

- **One origin for the browser.** The Next.js server proxies `/api/*` and `/uploads/*` to the
  backend (`BACKEND_URL`, fixed at **build** time). So there is no CORS configuration, and the
  session cookies are first-party and `SameSite=Strict`.
- **Backend** (`backend/`): axum handlers in `src/routes/`, authentication in `src/auth.rs`, and
  database access in `src/db.rs` (an SQLx `PgPool` with typed helpers). Migrations are embedded
  in the binary and applied on startup. The same binary is a small CLI:
  `serve | migrate | check-db | verify | healthcheck`.
- **Frontend** (`frontend/`): client components calling the API through `src/lib/api.ts`. It
  sends cookies and the CSRF header automatically.
- **Health checks:**
  - `GET /api/health`: liveness; the process is up.
  - `GET /api/health/ready`: readiness; the database answers.
  - `GET /healthz`: the frontend is up.
  - Both images have a Docker `HEALTHCHECK`. No curl is needed: the backend binary probes itself.

---

## Quick start (Docker)

Requirements: Docker Desktop (or Docker Engine with the compose plugin) and Node.js 20+ for the
helper scripts.

```bash
cp .env.example .env          # Windows: copy .env.example .env
# Edit .env: set POSTGRES_PASSWORD (openssl rand -hex 24), JWT_SECRET (openssl rand -hex 32),
# and ADMIN_EMAIL / ADMIN_PASSWORD for the first admin account.
docker compose up -d --build  # PostgreSQL + API + frontend
```

Open **http://localhost:3000** and sign in with `ADMIN_EMAIL` / `ADMIN_PASSWORD`. On first start
the backend applies the migrations and creates that admin.

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
| Start / rebuild everything | `docker compose up -d --build` |
| Stop (keeps data) | `docker compose down` |
| Logs | `docker compose logs -f backend` (or `frontend`, `db`) |
| Start only PostgreSQL | `npm run db:up` |
| SQL shell | `docker compose exec db psql -U hourwise -d hourwise` |
| Apply migrations (normally automatic) | `npm run db:migrate` |
| Integrity report (read-only) | `npm run db:verify` |
| **Verified backup** | `npm run backup` → `backups/hourwise-<timestamp>.dump` |
| **Reset the local dev database** | `npm run db:reset -- --yes` (deletes only the `hourwise_postgres_data` volume) |

Local data lives in Docker volumes: `hourwise_postgres_data` (PostgreSQL) and `hourwise_uploads`
(avatars). `docker compose down -v` would delete **both**. Prefer `npm run db:reset`, which only
deletes the database and asks for `--yes`. Volumes left over from earlier versions
(`hourwise_pgdata`, `hourwise_dbdata`) are not used or touched by this version.

---

## Local development (hot reload)

Run PostgreSQL in Docker and the apps on your machine:

```bash
npm run db:up                             # PostgreSQL on 127.0.0.1:5432
cp backend/.env.example backend/.env      # set DATABASE_URL (same password as POSTGRES_PASSWORD) and JWT_SECRET
npm run dev:backend                       # API on http://localhost:8080; applies migrations on start
npm --prefix frontend install
npm run dev:frontend                      # http://localhost:3000, proxies /api to :8080
```

- `npm run dev:backend` uses your local Rust toolchain if there is one, otherwise it runs
  `cargo run` in the `rust:1-bookworm` Docker image. Rust needs a C toolchain: MSVC Build Tools on
  Windows, Xcode CLT on macOS, build-essential on Linux. On Windows, install it with
  `winget install Rustlang.Rustup`.
- Stop the full stack first (`docker compose stop backend frontend`) because it uses the same
  ports, or change `BACKEND_PORT` / `FRONTEND_PORT`.
- If the backend runs elsewhere, start the frontend with `BACKEND_URL=http://host:port`.

---

## Configuration

### Root `.env` (Docker Compose)

| Variable | Default | Description |
| --- | --- | --- |
| `POSTGRES_USER` / `POSTGRES_DB` | `hourwise` | Local database user and name. |
| `POSTGRES_PASSWORD` | *(required)* | Local database password. Use letters and digits (it is put into a URL). |
| `POSTGRES_PORT` | `5432` | Host port, bound to `127.0.0.1` only. |
| `DATABASE_MAX_CONNECTIONS` | `10` | Pool size per backend instance. |
| `JWT_SECRET` | *(required)* | Signs session cookies. At least 32 characters. Changing it signs everyone out. |
| `JWT_TTL_HOURS` | `168` | Session lifetime. |
| `COOKIE_SECURE` | `true` | `Secure` cookies. Browsers accept them on `http://localhost`; set `false` only to test over plain HTTP on another host. |
| `LOGIN_MAX_ATTEMPTS` / `LOGIN_LOCKOUT_SECS` | `5` / `900` | Failed logins per email **from one client address** before that pair is locked for `LOGIN_LOCKOUT_SECS`. |
| `LOGIN_MAX_ATTEMPTS_PER_IP` | `20` | Failed logins per client address across all emails (password spraying). |
| `PASSWORD_HASH_CONCURRENCY` | `4` | Argon2 operations at the same time (~19 MB each). Requests that wait more than 5 s get `503`. |
| `TRUSTED_PROXIES` | *(empty)* | Comma-separated IPs/CIDR ranges whose `X-Forwarded-For` is trusted. Keep empty locally; for Dokploy see `docs/DEPLOYMENT.md`. |
| `ADMIN_EMAIL` / `ADMIN_PASSWORD` / `ADMIN_NAME` | – | First admin, created on startup only when no admin exists. |
| `BACKEND_PORT` / `FRONTEND_PORT` | `8080` / `3000` | Host ports. The backend is bound to `127.0.0.1`. |

### Backend environment (`backend/.env` for `npm run dev:backend`, or the container environment)

| Variable | Default | Description |
| --- | --- | --- |
| `DATABASE_URL` | *(required)* | `postgres://user:password@host:5432/dbname`. Add `?sslmode=require` for a database reached over the internet. |
| `DATABASE_MAX_CONNECTIONS` | `10` | Maximum pooled connections. Keep (replicas × this) below PostgreSQL's `max_connections` (100 by default). |
| `DATABASE_ACQUIRE_TIMEOUT_SECS` | `5` | How long a request waits for a free connection before failing with `503`. |
| `DATABASE_STATEMENT_TIMEOUT_SECS` | `30` | Server-side limit per SQL statement. |
| `BIND_ADDR` | `0.0.0.0:8080` | Listen address (`127.0.0.1:8080` in `backend/.env`). |
| `UPLOAD_DIR` | `./uploads` (`/data/uploads` in Docker) | Avatar storage. Needs a persistent volume in production. |
| `CORS_ORIGIN` | – | Only if a browser calls the API directly. Not needed with the Next.js proxy. |
| `RUST_LOG` | `hourwise_backend=info,tower_http=info` | Log filter. |
| `JWT_*`, `COOKIE_SECURE`, `LOGIN_*`, `ADMIN_*` | | As above. |

### Frontend

| Variable | When | Description |
| --- | --- | --- |
| `BACKEND_URL` | **build time** (`--build-arg`, or the env of `npm run build` / `npm run dev`) | Where the Next.js server proxies `/api` and `/uploads`. `http://backend:8080` in Compose, the backend's private service name in Dokploy. |

Secrets are only ever read from the environment. The database password, `JWT_SECRET` and
`ADMIN_PASSWORD` are never logged: the config's debug output redacts them, and only
`user@host:port/db` is printed about the database.

---

## Database

### Schema

| Table | Columns |
| --- | --- |
| `users` | `id UUID` PK, `email` (unique, lower-case enforced by a `CHECK`), `name`, Argon2 `password_hash`, `role` (`admin`/`member`), `job_title`, `avatar_url`, `is_active BOOLEAN`, `session_epoch BIGINT`, `created_at`/`updated_at TIMESTAMPTZ` |
| `projects` | `id UUID` PK, `name`, `client`, `description`, `color`, `is_archived BOOLEAN`, timestamps |
| `project_members` | (`project_id`, `user_id`) composite PK, both foreign keys `ON DELETE CASCADE`, `added_at` |
| `time_entries` | `id UUID` PK, `user_id`/`project_id` FKs `ON DELETE CASCADE`, `entry_date DATE`, `minutes INTEGER CHECK (1..1440)`, `description`, timestamps |

Indexes: `time_entries (user_id, entry_date)` (timesheets and the daily cap),
`(project_id, entry_date)` (project reports), `(entry_date)` (organisation-wide reports and
dashboard stats), and `project_members (user_id)`.

- **IDs** are UUIDs generated by the application.
- **Dates** are `DATE`. Timestamps are `TIMESTAMPTZ`, stored in UTC with microsecond precision.
- **Durations** are whole minutes, so all totals are exact.
- **The 24-hour daily cap** is enforced in a transaction that first locks the user's row
  (`SELECT … FOR UPDATE`). Simultaneous submissions for the same user are serialised, and other
  users aren't blocked.
- **Deleting** a project or user removes its memberships and entries in one transaction.
  Removing a member keeps their entries.

### Migrations

- **Location:** `backend/migrations/NNNN_description.sql`, embedded at compile time with
  `sqlx::migrate!` and applied automatically on startup (or with `hourwise-backend migrate` /
  `npm run db:migrate`).
- **Tracking:** applied versions and checksums are stored in `_sqlx_migrations`. Each migration
  runs in its own transaction, and a PostgreSQL advisory lock makes simultaneous startups safe.
- **Safety:** the backend refuses to start if an applied migration was edited, or if the database
  has a migration this build doesn't know (e.g. an older backend against a newer database).
- **Never edit an applied migration.** Add `0002_….sql` and so on.
- **Before migrating production,** take a backup.

### Starting fresh

The previous Turso databases held no data that needs to be kept, so there is no data migration.
A new PostgreSQL database is initialised automatically: start the backend with `DATABASE_URL`
pointing at an empty database and it creates the schema and the first admin
(`ADMIN_EMAIL` / `ADMIN_PASSWORD`). Your Turso databases were not modified. Delete them in the
Turso dashboard whenever you like.

---

## Backups and recovery

- **Local development:** `npm run backup`.
  - It runs `pg_dump --format=custom` inside the `db` container and writes it to `backups/`
    (gitignored) with a SHA-256 checksum.
  - It then restores the dump into a **throwaway** PostgreSQL container and checks that every
    table's row count matches. Only then does it report `OK`.
  - `npm run backup -- --keep 30` also prunes old files.
- **Production (Dokploy):** scheduled `pg_dump` backups to off-server S3-compatible storage,
  volume backups for avatars, retention, monthly restore tests and the restore procedure are all
  in [docs/DEPLOYMENT.md → Backups and restore](docs/DEPLOYMENT.md#7-backups-and-restore).
- **Manual restore** of a local dump, into a database that must be empty:

  ```bash
  docker compose exec -T db pg_restore -U hourwise -d hourwise --no-owner --exit-on-error < backups/<file>.dump
  ```

- **Treat backups as sensitive:** they contain emails, password hashes and time logs.

---

## Testing

| Command | What runs | Database |
| --- | --- | --- |
| `npm test` | All Rust tests + all Vitest tests | disposable test server (below) |
| `npm run test:backend` | Rust: unit, database, API and security tests | disposable test server |
| `npm run test:unit` | Rust unit tests only | none |
| `npm run test:integration` | Rust database + API + security tests | disposable test server |
| `npm run test:frontend` | Vitest + React Testing Library (API mocked) | none |
| `npm run test:e2e` | Playwright (Chromium) against an isolated stack on ports 3100/8180 | throwaway tmpfs PostgreSQL |
| `npm run test:all` | All of the above | |

**Rust test database safety.** `npm run test:backend` starts a disposable PostgreSQL server
(`docker-compose.test.yml`: tmpfs, `127.0.0.1:55432`, marked `hourwise.disposable=on`), runs the
tests and removes the server again. Each test creates its own `hourwise_test_<random>` database
and drops it when it finishes. The harness (`backend/tests/common/mod.rs`) refuses to run unless
all of these hold:

- The server comes **only** from `HOURWISE_TEST_DATABASE_URL`. It never falls back to
  `DATABASE_URL`.
- The server is not the same server as any protected database:
  - `DATABASE_URL` from your shell, the root `.env` or `backend/.env`
  - the Compose development database
  - the comparison normalises `postgres://`/`postgresql://`, host case, `localhost`/`127.0.0.1`/
    `::1`/`host.docker.internal`, default ports, encoding and credentials
- The server reports `hourwise.disposable = on`, which production servers don't.
- Each new database is empty before migrations run.
- Only databases it created itself (with the test prefix) are dropped.
- Passwords are never printed.

To use another disposable server, set `HOURWISE_TEST_DATABASE_URL` to its maintenance database,
and start that server with `-c hourwise.disposable=on`.

Coverage includes:

- connectivity and pooling
- migrations: fresh, repeated, concurrent, tampered and unknown
- the schema: tables, indexes, column types
- constraints: unique, check, not null, foreign keys, cascades
- transactions and rollback
- per-user row locking
- the concurrent 24h-cap race: 16 simultaneous requests and exactly one wins
- pool exhaustion and statement timeouts returning `503`
- authentication, cookies, CSRF, session revocation and rate limiting
- admin authorisation for every endpoint, and members not seeing each other's data
- SQL-injection and malicious-input handling
- timesheet create, read, update and delete, and reports
- health and readiness endpoints

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

## Deploying to production

See **[docs/DEPLOYMENT.md](docs/DEPLOYMENT.md)**:

- Dokploy prerequisites and installation
- the PostgreSQL service
- backend and frontend applications (Dockerfile paths, build arguments, environment, volume,
  health checks, update and rollback settings)
- domain and HTTPS
- backups and restore
- deployment order, automatic deploys, rollbacks
- a production checklist and troubleshooting

### Docker images

```bash
docker build -t hourwise-backend ./backend
docker build --build-arg BACKEND_URL=http://<backend-host>:8080 -t hourwise-frontend ./frontend
```

**Backend image:**
- multi-stage release build with cached dependencies
- `debian:bookworm-slim` runtime with only `ca-certificates`
- non-root user `hourwise` (uid 10001)
- graceful shutdown on SIGTERM, which also closes the pool
- `HEALTHCHECK` via `hourwise-backend healthcheck`
- no secrets baked in

**Frontend image:**
- `npm ci` with the existing `package-lock.json`
- Next.js `standalone` output, so no dev dependencies ship
- non-root user `nextjs`
- `HEALTHCHECK` on `/healthz`

---

## Security notes


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


- **Network exposure:**
  - Locally, PostgreSQL and the API are published on `127.0.0.1` only.
  - In production, only the frontend has a public domain. The API and database are reachable
    only on Dokploy's private network, and the database has no external port.
- **HTTPS is required in production** because the cookies are `Secure`. Dokploy's Traefik
  terminates TLS with Let's Encrypt.
- **Response headers:** the frontend sends `X-Content-Type-Options`, `X-Frame-Options`,
  `Referrer-Policy` and `Permissions-Policy` on every page.
- **Known limits:**
  - Login rate limits are per email+client address and per client address, so one client can't
    lock an account for everyone, and spraying from one address is blocked. The client address
    comes from `X-Forwarded-For` only when the connection comes from `TRUSTED_PROXIES`. Many
    addresses together can still guess slowly. Argon2 work is capped (`PASSWORD_HASH_CONCURRENCY`)
    so a login flood gets `503` instead of exhausting memory.
  - The limiter is in memory, per backend instance, and reset by a restart. Run one backend replica.
  - Logout clears the cookies in that browser only. A stolen cookie is valid until it expires
    (`JWT_TTL_HOURS`), unless the user's password is changed or the user is deactivated.
  - Avatars are served without authentication to anyone who knows the random file name.

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
├── docker-compose.yml        # local dev: db (PostgreSQL) + backend + frontend
├── docker-compose.test.yml   # disposable PostgreSQL for the Rust tests
├── docker-compose.e2e.yml    # isolated stack for the Playwright tests
├── docs/DEPLOYMENT.md        # Dokploy deployment, backups, checklist
├── .env.example              # copy to .env
├── package.json              # repo-level scripts (no dependencies)
├── scripts/
│   ├── cargo.mjs             # runs cargo locally or in Docker; test-database guard
│   ├── test-backend.mjs      # starts the disposable test server and runs cargo test
│   ├── backup.mjs            # verified pg_dump backups of the local database
│   ├── db-reset.mjs          # resets only the local dev database volume
│   └── e2e.mjs               # starts the isolated stack and runs Playwright
├── .github/workflows/ci.yml
├── backend/                  # Rust API
│   ├── Dockerfile
│   ├── migrations/           # PostgreSQL migrations (embedded with sqlx::migrate!)
│   ├── src/
│   │   ├── main.rs           # CLI: serve | migrate | check-db | verify | healthcheck
│   │   ├── lib.rs            # app + state construction (shared with tests)
│   │   ├── db.rs             # PgPool, settings, args!, Record/FromRecord, destinations
│   │   ├── migrate.rs        # migration runner
│   │   ├── verify.rs         # read-only integrity report
│   │   ├── config.rs         # env configuration (secrets redacted in Debug)
│   │   ├── auth.rs           # Argon2 (+ concurrency limit), cookies, CSRF, rate limiting, extractors
│   │   ├── client_ip.rs      # client address for rate limiting (TRUSTED_PROXIES)
│   │   ├── error.rs          # AppError, SQLSTATE mapping, JSON responses
│   │   ├── models.rs         # row / response types
│   │   └── routes/           # health, auth, me, projects, entries, admin
│   └── tests/                # common (harness), db, api, security
└── frontend/                 # Next.js app
    ├── Dockerfile
    ├── e2e/                  # Playwright specs
    └── src/
        ├── __tests__/        # Vitest + React Testing Library
        ├── app/              # login, (app)/ member UI, admin/ UI, healthz route
        ├── components/       # UI kit, EntryModal, SummaryReport, PeriodNav, ...
        └── lib/              # api client, auth context, csv, hooks, time helpers, types
```

---

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| `docker compose up`: `Set POSTGRES_PASSWORD in .env` | Copy `.env.example` to `.env` and fill it in. |
| Backend exits with `DATABASE_URL must be set` (`npm run dev:backend`) | Create `backend/.env` from `backend/.env.example`. |
| `database not reachable (attempt n/12)` | PostgreSQL isn't running (`npm run db:up`), or the password in `DATABASE_URL` doesn't match `POSTGRES_PASSWORD`. Percent-encode `@ : / #` in passwords. |
| `password authentication failed` after changing `POSTGRES_PASSWORD` | The password is set only when the volume is first created. Change it back, change it in SQL (`ALTER USER hourwise PASSWORD '…'`), or reset the dev database. |
| Port 5432 already in use | Another PostgreSQL is running locally. Set `POSTGRES_PORT=5433` in `.env` and the matching port in `backend/.env`. |
| `migration N was modified after it was applied` | Restore the original migration file and add a new migration instead. |
| `npm run test:backend`: `refusing to run: … same server as a protected database` | `HOURWISE_TEST_DATABASE_URL` points at a real server. Unset it to use the disposable one. |
| `npm run test:backend`: `not marked as a disposable test server` | Use `docker-compose.test.yml`, or start your test server with `-c hourwise.disposable=on`. |
| `cargo: The term 'cargo' is not recognized` | Use the `npm run …` scripts (they fall back to Docker), or install Rust with `winget install Rustlang.Rustup`. |
| `/api/...` returns 502 from the frontend | The backend is down or `BACKEND_URL` was wrong at build time. Rebuild the frontend. |
| Signed out right after login | Not served over HTTPS (production), or `COOKIE_SECURE=true` on a non-localhost HTTP address. |
