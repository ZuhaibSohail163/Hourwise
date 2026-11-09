# CLAUDE.md

Guidance for Claude Code (and humans) working in this repository.

## What this is
Hourwise is a team time-tracking web app (Harvest-like). There are two roles:
- **member**: logs time (hours and minutes, with notes) to projects they belong to, and views
  day/week/month reports.
- **admin**: also manages projects, project membership and users, and sees everyone's time and
  work through a separate `/admin` interface.

## Layout
- `backend/`: Rust API (axum 0.8, **libsql** 0.9 for Turso Cloud or a local SQLite file, argon2,
  jsonwebtoken). It's a lib + bin crate: `src/lib.rs` builds the app and state (shared with the
  tests); `src/main.rs` is the CLI (`serve | migrate | check-db | verify | backup | restore`).
  - `src/db.rs`: `Db` (one connection per request via `state.db.conn()`), the `args![]` macro, the
    `Bind` trait, `Record` row accessors, `FromRecord`, `query_all/opt/one/value/i64`, `execute`,
    and `write_tx` (`BEGIN IMMEDIATE`).
  - `src/migrate.rs`: embedded migrations list + checksums. `src/backup.rs`: export/import/verify.
  - `src/routes/mod.rs` defines the router and shared helpers (`date_range`, `clean`, validators).
  - `src/routes/admin.rs` holds every `/api/admin/*` endpoint.
  - `src/auth.rs`: Argon2, session cookies, CSRF, `LoginLimiter`, and the `AuthUser` / `AdminUser`
    extractors. **Every protected handler must take one of them**; `AdminUser` is what enforces
    admin-only access, and `AuthUser` enforces CSRF on non-GET requests.
  - `migrations/`: SQLite-dialect SQL, embedded in the binary.
  - `tests/`: `common/mod.rs` (TestApp harness: temp DB per test, cookie sessions), `db.rs`,
    `api.rs`, `security.rs`, `turso_cloud.rs` (`#[ignore]`, needs an empty test DB).
- `frontend/`: Next.js 16 App Router, React 19, TypeScript, Tailwind CSS v4 (configured in CSS with `@theme` in `globals.css`).
  - `src/app/(app)/` is the member interface (top nav). `src/app/admin/` is the admin interface (dark sidebar). Each `layout.tsx` guards its own role.
  - All pages are client components that call the API through `src/lib/api.ts` (`api<T>(path, {method, body, query, form})`).
    It sends cookies and the `X-CSRF-Token` header automatically. There is no token in JS or localStorage.
  - `src/components/SummaryReport.tsx` is the shared report (totals, charts, work log, CSV via `lib/csv.ts`).
  - `src/lib/time.ts` has date and duration helpers. Weeks start Monday (`WEEK_OPTS`).
  - Tests: `src/__tests__/*.test.ts(x)` (Vitest + RTL, `src/test/` holds setup and fixtures) and `e2e/` (Playwright).
- `docker-compose.yml`: `backend`, `frontend` (there is no database container: Turso Cloud or a local file in the `dbdata` volume).
  `docker-compose.e2e.yml`: isolated tmpfs stack for Playwright.

## Commands
**Before finishing any change, run `npm run format`, `npm run check` and `npm test` from the repo root.**
All must pass with zero warnings, because CI enforces the same checks.
```bash
npm run format                        # rustfmt + prettier (repo root)
npm run lint                          # clippy -D warnings + eslint --max-warnings=0
npm run check                         # format check + lints + tsc
npm test                              # cargo test (unit + db + api + security) + vitest
npm run test:e2e                      # Playwright on the isolated stack (ports 3100/8180)
npm run test:turso                    # real Turso; needs HOURWISE_TEST_TURSO_URL/TOKEN for an EMPTY db
npm run backup                        # verified logical backup -> backups/
docker compose up -d --build          # full stack at http://localhost:3000 (local DB unless TURSO_* set)
docker compose exec backend hourwise-backend verify   # integrity report
npm run dev:backend                   # API on :8080 (needs backend/.env); Docker if no Rust
cd frontend && npm run dev            # proxies /api -> http://localhost:8080
```
Rust may not be installed on the host. `node scripts/cargo.mjs <args>` (which the root npm scripts use)
falls back to running cargo in the `rust:1-bookworm` Docker image, e.g. `node scripts/cargo.mjs test --test api`.

## Conventions and gotchas
- **SQL is SQLite/libSQL**, sent through `db::query_*` / `db::execute` with numbered placeholders
  (`?1`, `?2`, and the same number can repeat) and values from `args![...]`. Never format user
  input into SQL. Table names in `backup.rs` come from a constant list only.
- **Storage formats** (enforced by `CHECK`s in STRICT tables): UUIDs are hyphenated lower-case
  text, booleans are 0/1, dates are `YYYY-MM-DD` text (which compares correctly), timestamps are
  RFC 3339 UTC with milliseconds. Set `created_at`/`updated_at` from Rust with `db::now_ts()`;
  don't use SQL `now()` or `CURRENT_TIMESTAMP` (wrong format).
- Map rows by implementing `FromRecord` (in `models.rs`) and using `r.text/opt_text/i64/bool/uuid/date/ts("col")`.
  `SUM()`/`COUNT()` come back as integers. Use `COALESCE(SUM(x), 0)`.
- **Don't rely on `PRAGMA foreign_keys` or `ON DELETE CASCADE`** for correctness. Over Turso's
  HTTPS protocol the pragma isn't guaranteed per request. Delete children explicitly in a
  `write_tx` (see `delete_user` / `delete_project`) and check referenced rows exist before
  inserting (see `add_members`). Local connections enable foreign keys as a safety net.
- Read-check-write logic (like the daily cap) must run inside `db::write_tx` so it is atomic.
- Durations are stored as **integer minutes** (`time_entries.minutes`, 1–1440). The UI collects
  hours and minutes and sends `minutes`. The API also caps a user's total per day at 24h.
- Business rules live in the backend, not only the UI:
  - members can only create or edit entries on non-archived projects they are members of
  - users can't change their own email (only `PATCH /api/admin/users/{id}` can)
  - admins can't demote, deactivate or delete themselves
- Removing a project member keeps their entries. Deleting a project or user deletes their entries.
- Emails are stored lower-cased. Normalize them before lookups and inserts.
- **Sessions:** cookies only (`hw_session` HttpOnly + `hw_csrf`). Bump `users.session_epoch` to
  revoke sessions (password change/reset, deactivation). Don't add Bearer-token auth or put
  tokens in localStorage.
- Errors: return `AppError`. It serializes to `{"error": "..."}` and the frontend shows `error`.
  `libsql` errors are mapped in `error.rs` (constraint → 400/409, connection → 503).
- Adding a schema change means adding a **new** `migrations/000N_description.sql` **and** appending
  it to `MIGRATIONS` in `src/migrate.rs`. Never edit an applied migration (the checksum check fails).
  If a column is added to a table, `backup.rs` picks it up automatically via `pragma_table_info`.
- Secrets (`TURSO_AUTH_TOKEN`, `JWT_SECRET`) come only from the environment. Never log or commit
  them; `DbTarget::describe()` is the only thing logged about the database.
- Tests must use isolated data: `TestApp::new()` in backend tests, mocked `api` in Vitest, the e2e
  stack for Playwright. Never point tests at a real database. `turso_cloud.rs` refuses non-empty databases.
- Next.js `rewrites` are resolved at **build time**, so `BACKEND_URL` must be set during `next build`.
- With Next 16, use `useParams()` in client pages rather than the `params` prop.
- **Data fetching:** use `useApiData(path, query)` from `src/lib/hooks.ts`. Don't use `useEffect` +
  `setState`, because the `react-hooks/set-state-in-effect` lint rule rejects it. Use `reload()` after
  mutations and `mutate(fn)` for local updates. Report pages use `usePeriod()` for day/week/month state.
- **Modals with forms:** put the form in a child component rendered inside `<Modal>`. It mounts only
  while open, so initialize its state with `useState(initial)` instead of resetting it in an effect.
- In Vitest, write `beforeEach(() => { mock.mockReset(); })` with braces: a returned function is
  treated as a cleanup hook and called.
- Don't silence lint rules with `eslint-disable` unless there's a real reason, and add a comment
  explaining it (see the two intentional `window.location` redirects in `lib/api.ts` and `lib/auth.tsx`).
- Keep the visual language consistent: use the components in `components/ui.tsx` (Button, Card,
  Field, Modal, Badge, StatCard...) and the `brand-*` / `ink-*` colour tokens.
