# Hourwise

Hourwise is a self-hosted team time tracker, loosely modelled on the Harvest timesheet.
Team members sign in with email and password and log hours and minutes per day against the
projects they belong to. Admins get their own interface where they create projects, manage
members, and see who worked on what, for how long and what they did.

- **Backend:** Rust ([axum](https://github.com/tokio-rs/axum) + [sqlx](https://github.com/launchbadge/sqlx)) REST API
- **Frontend:** Next.js 16 (App Router, React 19, TypeScript, Tailwind CSS 4)
- **Database:** PostgreSQL 16. It is bundled as a Docker container, and any hosted Postgres (Neon, Supabase, RDS…) also works.
- **Packaging:** Docker Compose. One command starts the whole stack.

---

## Features

### For everyone
- **Email + password sign-in.** Sessions use signed JWTs; passwords are hashed with Argon2.
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

## Quick start (Docker, recommended)

**Prerequisites:** [Docker Desktop](https://www.docker.com/products/docker-desktop/) (or Docker Engine + Compose v2) and Git.

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
   - `POSTGRES_PASSWORD`: the database password.

```bash
# 4. Build and start everything (the first build takes a few minutes while Rust compiles)
docker compose up -d --build
```

5. Open **http://localhost:3000** and sign in with `ADMIN_EMAIL` / `ADMIN_PASSWORD`.

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
| Stop | `docker compose down` |
| View logs | `docker compose logs -f backend` (or `frontend`, `db`) |
| Open a DB shell | `docker compose exec db psql -U hourwise hourwise` |
| Back up the DB | `docker compose exec db pg_dump -U hourwise hourwise > backup.sql` |
| Restore a backup | `docker compose exec -T db psql -U hourwise hourwise < backup.sql` |
| **Wipe all data** | `docker compose down -v` (deletes the database and avatar volumes) |

Data lives in two named Docker volumes: `pgdata` (database) and `uploads` (profile pictures).

---

## Local development (without Docker for the app)

You can still run Postgres in Docker and run the backend and frontend natively for faster iteration.

**Prerequisites:** [Rust](https://rustup.rs/) (stable), Node.js 20+ and Docker (for Postgres only).

```bash
# 1. Start only the database (uses POSTGRES_* from the root .env, or the defaults)
docker compose up -d db

# 2. Backend
cd backend
cp .env.example .env            # make DATABASE_URL match your db credentials; set JWT_SECRET
cargo run                       # http://localhost:8080 (migrations run automatically)

# 3. Frontend (new terminal)
cd frontend
npm install
npm run dev                     # http://localhost:3000, proxies /api to http://localhost:8080
```

If the backend runs somewhere other than `http://localhost:8080`, start the frontend with
`BACKEND_URL=http://host:port npm run dev`.

> If you also run the full stack with `docker compose up`, the containers use ports 8080 and 3000
> as well. Stop them first, or change `BACKEND_PORT` / `FRONTEND_PORT` in `.env`.

## Linting & formatting

| Tool | Scope | Config |
| --- | --- | --- |
| **rustfmt** | Rust formatting | `backend/rustfmt.toml` |
| **clippy** | Rust lints, with warnings treated as errors | `[lints]` in `backend/Cargo.toml` |
| **Prettier** (+ Tailwind class sorting) | TS/TSX/CSS/JSON formatting | `frontend/.prettierrc.json` |
| **ESLint** (Next.js core-web-vitals + TypeScript + React hooks rules) | TS/TSX lints | `frontend/eslint.config.mjs` |
| **TypeScript** | type checking | `frontend/tsconfig.json` |

Run these from the **repo root**. They work in PowerShell, cmd and bash, and you don't need
`npm install` at the root:

```bash
npm run format          # auto-format backend + frontend
npm run lint            # clippy + eslint, fail on any warning
npm run lint:fix        # apply automatic lint fixes
npm run check           # what CI runs: format check + lints + type check
```

If Rust isn't installed locally, the backend commands automatically run `cargo` inside the
`rust:1-bookworm` Docker image (via `scripts/cargo.mjs`). Set `HOURWISE_CARGO=docker` to force that.

You can also run each side on its own:

```bash
cd backend  && cargo fmt && cargo clippy --all-targets -- -D warnings
cd frontend && npm run format && npm run lint && npm run typecheck
```

`.github/workflows/ci.yml` runs the same checks plus both builds on every push and pull request.
`.editorconfig` and `.gitattributes` keep indentation and LF line endings consistent across
editors and Windows checkouts. With the Prettier and ESLint editor extensions installed,
VS Code and JetBrains pick up these configs automatically, so you can enable format-on-save.

---

## Configuration

### Root `.env` (read by Docker Compose)

| Variable | Default | Description |
| --- | --- | --- |
| `JWT_SECRET` | *(required)* | Secret for signing login tokens. At least 32 characters. Changing it signs everyone out. |
| `JWT_TTL_HOURS` | `168` | How long a login stays valid. |
| `ADMIN_EMAIL` / `ADMIN_PASSWORD` / `ADMIN_NAME` | – | Initial admin, created on startup only when no admin exists. |
| `POSTGRES_USER` / `POSTGRES_PASSWORD` / `POSTGRES_DB` | `hourwise` | Credentials for the bundled Postgres container. |
| `DATABASE_URL` | built from `POSTGRES_*` | Set this to use a **hosted** Postgres instead of the bundled one. |
| `POSTGRES_PORT` / `BACKEND_PORT` / `FRONTEND_PORT` | `5432` / `8080` / `3000` | Host ports. |

### Backend environment (`backend/.env` when running with `cargo run`)

| Variable | Default | Description |
| --- | --- | --- |
| `DATABASE_URL` | *(required)* | Postgres connection string. |
| `JWT_SECRET` | *(required)* | As above. |
| `BIND_ADDR` | `0.0.0.0:8080` | Address the API listens on. |
| `UPLOAD_DIR` | `./uploads` | Where profile pictures are stored. |
| `CORS_ORIGIN` | *(unset)* | Only needed if a browser calls the API directly instead of through the Next.js proxy. |
| `RUST_LOG` | `hourwise_backend=info,tower_http=info` | Log level. |

### Using a hosted database (Neon / Supabase / RDS)

1. Create a Postgres database with your provider and copy its connection string.
2. Put it in `.env`, e.g. `DATABASE_URL=postgres://user:pass@host/db?sslmode=require`.
3. Run `docker compose up -d --build backend frontend`. The bundled `db` service still starts
   because it's a dependency, but it isn't used. The backend creates its tables on first start.

---

## Architecture

```
Browser ──► Next.js (port 3000) ──/api/*, /uploads/*──► Rust API (port 8080) ──► PostgreSQL
            pages + proxy                                axum + sqlx
```

- The browser only talks to the Next.js server, which proxies `/api/*` and `/uploads/*` to the
  Rust backend (`rewrites` in `frontend/next.config.ts`). This means there's no CORS setup.
  `BACKEND_URL` is resolved **at build time**; Docker passes it as a build arg.
- Auth: `POST /api/auth/login` returns a JWT that the frontend stores in `localStorage` and
  sends as `Authorization: Bearer …`. On every request the backend reloads the user, so
  deactivation and role changes apply immediately.
- Migrations live in `backend/migrations/`, are embedded in the binary and run automatically on startup.

### Data model

| Table | Purpose |
| --- | --- |
| `users` | email (unique, lower-cased), name, Argon2 hash, role (`admin`/`member`), job title, avatar, active flag |
| `projects` | name, client, description, colour, archived flag |
| `project_members` | which users may log time to which projects |
| `time_entries` | user, project, date, minutes (1–1440), description |

### REST API

All endpoints are under `/api`. Everything except `/health` and `/auth/login` requires a bearer token.

| Method & path | Who | Description |
| --- | --- | --- |
| `POST /auth/login` | public | `{email, password}` → `{token, user}` |
| `GET /me` | user | Current user |
| `PATCH /me` | user | `{name?, job_title?}` (email can't be changed here) |
| `PUT /me/password` | user | `{current_password, new_password}` |
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
├── docker-compose.yml        # db + backend + frontend
├── .env.example              # copy to .env
├── package.json              # repo-level lint/format scripts (no dependencies)
├── scripts/cargo.mjs         # runs cargo locally, or in Docker if Rust isn't installed
├── .github/workflows/ci.yml  # format + lint + typecheck + build on every push/PR
├── backend/                  # Rust API
│   ├── Cargo.toml
│   ├── Dockerfile
│   ├── migrations/           # SQL migrations (embedded, run at startup)
│   └── src/
│       ├── main.rs           # startup, router, admin seeding
│       ├── config.rs         # env configuration
│       ├── auth.rs           # Argon2, JWT, AuthUser/AdminUser extractors
│       ├── error.rs          # AppError → JSON responses
│       ├── models.rs         # DB row / response types
│       └── routes/           # auth, me, projects, entries, admin
└── frontend/                 # Next.js app
    ├── Dockerfile
    ├── next.config.ts        # API proxy (rewrites)
    └── src/
        ├── app/
        │   ├── login/        # sign in
        │   ├── (app)/        # member interface: time, reports, profile
        │   └── admin/        # admin interface: overview, projects, users, reports
        ├── components/       # UI kit, EntryModal, SummaryReport, PeriodNav, ...
        └── lib/              # api client, auth context, time helpers, types
```

---

## Production notes

- Use a strong `JWT_SECRET` and database password, and put the app behind HTTPS (for example
  Caddy, Nginx or Traefik in front of port 3000). Only port 3000 needs to be public; you can
  remove the `ports:` mappings for `db` and `backend` in `docker-compose.yml`.
- Back up the `pgdata` and `uploads` volumes regularly (see the commands above).
- Weeks start on Monday. Dates are calendar dates (no time zone), so an entry logged for "Tuesday"
  stays on Tuesday for everyone.

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| `Set JWT_SECRET in .env` when running compose | Create `.env` from `.env.example` and set `JWT_SECRET`. |
| Backend exits with `JWT_SECRET must be at least 32 characters` | Use a longer secret. |
| Can't log in as admin after changing `ADMIN_PASSWORD` | The seed only runs when no admin exists. Reset with `docker compose down -v` (**deletes data**) or update the hash in the DB. |
| "Port is already allocated" | Change `POSTGRES_PORT`, `BACKEND_PORT` or `FRONTEND_PORT` in `.env`. |
| Frontend shows network errors in dev | Make sure the backend is running on `http://localhost:8080` or set `BACKEND_URL`. |
| A member sees no projects in "Track time" | An admin needs to add them to an active project. |
