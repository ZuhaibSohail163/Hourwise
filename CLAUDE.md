# CLAUDE.md

Guidance for Claude Code (and humans) working in this repository.

## What this is
Hourwise is a team time-tracking web app (Harvest-like). There are two roles:
- **member**: logs time (hours and minutes, with notes) to projects they belong to, and views
  day/week/month reports.
- **admin**: also manages projects, project membership and users, and sees everyone's time and
  work through a separate `/admin` interface.

## Layout
- `backend/`: Rust API (axum 0.8, **SQLx 0.8 + PostgreSQL**, argon2, jsonwebtoken). It's a lib + bin
  crate: `src/lib.rs` builds the app and state (shared with the tests); `src/main.rs` is the CLI
  (`serve | migrate | check-db | verify | healthcheck [--ready]`).
  - `src/db.rs`: `Db` (a `PgPool`; one pooled connection per request via `state.db.conn()`),
    `DbSettings` (`DATABASE_URL` + pool limits; the password never appears in Debug or logs), the
    `args![]` macro, `Record` row accessors, `FromRecord`, `query_all/opt/one/scalar/i64`, `execute`,
    `write_tx`, `lock_user` (`SELECT … FOR UPDATE`), and `destination()` (normalised host/port/db,
    used by the test guard).
  - `src/migrate.rs`: `sqlx::migrate!` (checksums in `_sqlx_migrations`, advisory lock).
    `src/verify.rs`: read-only integrity report.
  - `src/routes/mod.rs` defines the router, the health endpoints and shared helpers (`date_range`,
    `clean`, validators).
  - `src/routes/admin.rs` holds every `/api/admin/*` endpoint.
  - `src/auth.rs`: Argon2, session cookies, CSRF, `LoginLimiter`, `HashLimiter`, and the `AuthUser` / `AdminUser`
    extractors. **Every protected handler must take one of them**; `AdminUser` is what enforces
    admin-only access, and `AuthUser` enforces CSRF on non-GET requests.
    In handlers, hash and verify passwords with `state.hashing.hash/verify` (the concurrency limit),
    never the bare `hash_password`/`verify_password` (startup and tests only).
  - `src/client_ip.rs`: `ClientIp` extractor. `X-Forwarded-For` is used only when the TCP peer is in
    `TRUSTED_PROXIES` (rightmost untrusted entry); otherwise the peer address. Never read that header elsewhere.
    The login limiter is keyed per email+IP and per IP, never per email alone (that would allow lockout abuse).
  - `migrations/`: PostgreSQL SQL, embedded in the binary.
  - `tests/`: `common/mod.rs` (TestApp harness: a fresh database per test on the disposable test
    server, cookie sessions, production guard), `db.rs`, `api.rs`, `security.rs`.
- `frontend/`: Next.js 16 App Router, React 19, TypeScript, Tailwind CSS v4 (configured in CSS with `@theme` in `globals.css`).
  - `src/app/(app)/` is the member interface (top nav). `src/app/admin/` is the admin interface (dark sidebar). Each `layout.tsx` guards its own role. `src/app/healthz/route.ts` is the liveness probe.
  - All pages are client components that call the API through `src/lib/api.ts` (`api<T>(path, {method, body, query, form})`).
    It sends cookies and the `X-CSRF-Token` header automatically. There is no token in JS or localStorage.
  - `src/components/SummaryReport.tsx` is the shared report (totals, charts, work log, CSV via `lib/csv.ts`).
  - `src/lib/time.ts` has date and duration helpers. Weeks start Monday (`WEEK_OPTS`).
  - Tests: `src/__tests__/*.test.ts(x)` (Vitest + RTL, `src/test/` holds setup and fixtures) and `e2e/` (Playwright).
- `docker-compose.yml`: `db` (PostgreSQL 18, volume `postgres18_data` mounted at `/var/lib/postgresql`, port bound to 127.0.0.1), `backend`, `frontend`.
  `docker-compose.test.yml`: disposable tmpfs PostgreSQL for the Rust tests (127.0.0.1:55432, `hourwise.disposable=on`).
  `docker-compose.e2e.yml`: isolated tmpfs stack for Playwright. `docs/DEPLOYMENT.md`: Dokploy production setup.

## Commands
**Before finishing any change, run `npm run format`, `npm run check` and `npm test` from the repo root.**
All must pass with zero warnings, because CI enforces the same checks.
```bash
npm run format                        # rustfmt + prettier (repo root)
npm run lint                          # clippy -D warnings + eslint --max-warnings=0
npm run check                         # format check + lints + tsc
npm test                              # Rust tests (starts the disposable PostgreSQL) + vitest
npm run test:backend -- --test api    # one Rust test file
npm run test:e2e                      # Playwright on the isolated stack (ports 3100/8180)
npm run backup                        # verified pg_dump backup of the local db -> backups/
docker compose up -d --build          # full stack at http://localhost:3000
npm run db:up | db:migrate | db:verify | db:reset -- --yes   # local PostgreSQL helpers
npm run dev:backend                   # API on :8080 (needs backend/.env); Docker if no Rust
cd frontend && npm run dev            # proxies /api -> http://localhost:8080
```
Rust may not be installed on the host. `node scripts/cargo.mjs <args>` (which the root npm scripts use)
falls back to running cargo in the `rust:1-bookworm` Docker image, e.g. `node scripts/cargo.mjs test --lib`.

## Conventions and gotchas
- **SQL is PostgreSQL**, sent through `db::query_*` / `db::execute` with `$1`, `$2` placeholders
  (a number can repeat) and values from `args![...]`, passing `&mut conn` or `&mut tx`. Never format
  user input into SQL. Table names in `verify.rs` come from a constant list only.
- Bind values with the column's Rust type (`Uuid`, `NaiveDate`, `bool`, `i32` for `minutes`, `i64`).
  Binding a `&str` to a `uuid` or `date` column is a type error in PostgreSQL.
- **Types:** `UUID` ids (generated in Rust), `BOOLEAN`, `DATE`, `TIMESTAMPTZ`. Set
  `created_at`/`updated_at` from Rust with `db::now_ts()` (microsecond precision), so responses
  match what is stored.
- Map rows by implementing `FromRecord` (in `models.rs`) and using `r.text/opt_text/i64/bool/uuid/date/ts("col")`.
  `COUNT()` and `SUM(int)` return BIGINT; `r.i64` also accepts INT4. Use `COALESCE(SUM(x), 0)`, and
  `COUNT(*) FILTER (WHERE flag)` instead of summing booleans.
- Foreign keys cascade in the schema, and `delete_user` / `delete_project` also delete children
  explicitly in one `write_tx`. Keep both.
- Read-check-write logic (like the daily cap) must run inside `db::write_tx` **and** call
  `db::lock_user` first. PostgreSQL's default isolation (READ COMMITTED) doesn't prevent the race
  on its own: `daily_cap_holds_under_concurrent_requests` fails without the lock.
- Durations are stored as **integer minutes** (`time_entries.minutes`, 1–1440). The UI collects
  hours and minutes and sends `minutes`. The API also caps a user's total per day at 24h.
- Business rules live in the backend, not only the UI:
  - members can only create or edit entries on non-archived projects they are members of
  - users can't change their own email (only `PATCH /api/admin/users/{id}` can)
  - admins can't demote, deactivate or delete themselves
- Removing a project member keeps their entries. Deleting a project or user deletes their entries.
- Emails are stored lower-cased (a `CHECK` enforces it). Normalize them before lookups and inserts.
- **Sessions:** cookies only (`hw_session` HttpOnly + `hw_csrf`). Bump `users.session_epoch` to
  revoke sessions (password change/reset, deactivation). Don't add Bearer-token auth or put
  tokens in localStorage.
- Errors: return `AppError`. It serializes to `{"error": "..."}` and the frontend shows `error`.
  `sqlx` errors are mapped in `error.rs` by SQLSTATE (23505 → 409; 23503/23514/23502 → 400;
  timeouts, connection and pool errors → 503).
- A schema change is a **new** `migrations/000N_description.sql` (picked up by `sqlx::migrate!`;
  `build.rs` re-embeds on change). Never edit an applied migration, because the checksum check fails.
- Secrets (the password in `DATABASE_URL`, `JWT_SECRET`, `ADMIN_PASSWORD`) come only from the
  environment. Never log or commit them. `DbSettings::describe()` (`user@host:port/db`) is the only
  thing logged about the database, and `Config`'s Debug output redacts secrets.
- Tests must use isolated data: `TestApp::new()` in backend tests (its own database on the
  disposable server), mocked `api` in Vitest, and the e2e stack for Playwright. Never point tests at a
  real database. The harness only uses `HOURWISE_TEST_DATABASE_URL`, refuses servers that match a
  protected `DATABASE_URL`, requires `hourwise.disposable=on`, and drops only its own databases.
- Health: `/api/health` (liveness, no DB) and `/api/health/ready` (DB ping); the frontend has
  `/healthz`. In production only the frontend is public; it proxies to the backend's private name
  (`BACKEND_URL`, baked in at build time).
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
