# CLAUDE.md

Guidance for Claude Code (and humans) working in this repository.

## What this is
Hourwise is a team time-tracking web app (Harvest-like). There are two roles:
- **member**: logs time (hours and minutes, with notes) to projects they belong to, and views
  day/week/month reports.
- **admin**: also manages projects, project membership and users, and sees everyone's time and
  work through a separate `/admin` interface.

## Layout
- `backend/`: Rust API (axum 0.8, sqlx 0.8 with Postgres, argon2, jsonwebtoken). Entry point is `src/main.rs`.
  - `src/routes/mod.rs` defines the router and shared helpers (`date_range`, `clean`, validators).
  - `src/routes/admin.rs` holds every `/api/admin/*` endpoint.
  - `src/auth.rs` holds the `AuthUser` / `AdminUser` extractors. **Every protected handler must take
    one of them**; `AdminUser` is what enforces admin-only access.
  - `migrations/`: plain SQL. sqlx embeds them via `sqlx::migrate!` and runs them at startup.
- `frontend/`: Next.js 16 App Router, React 19, TypeScript, Tailwind CSS v4 (configured in CSS with `@theme` in `globals.css`).
  - `src/app/(app)/` is the member interface (top nav). `src/app/admin/` is the admin interface (dark sidebar). Each `layout.tsx` guards its own role.
  - All pages are client components that call the API through `src/lib/api.ts` (`api<T>(path, {method, body, query, form})`).
  - `src/components/SummaryReport.tsx` is the shared report (totals, charts, work log, CSV). Member and admin reports both use it.
  - `src/lib/time.ts` has date and duration helpers. Weeks start Monday (`WEEK_OPTS`).
- `docker-compose.yml`: `db` (postgres:16), `backend`, `frontend`.

## Commands
**Before finishing any change, run `npm run format` then `npm run check` from the repo root.**
Both must pass with zero warnings, because CI enforces the same checks.
```bash
npm run format                        # rustfmt + prettier (repo root)
npm run lint                          # clippy -D warnings + eslint --max-warnings=0
npm run check                         # format check + lints + tsc (what CI runs)
docker compose up -d --build          # full stack at http://localhost:3000
docker compose up -d db               # just Postgres, for native dev
cd backend && cargo run               # API on :8080 (needs backend/.env)
cd backend && cargo build && cargo clippy
cd frontend && npm run dev            # proxies /api -> http://localhost:8080
cd frontend && npm run typecheck && npm run build
```
Rust may not be installed on the host. `node scripts/cargo.mjs <args>` (which the root npm scripts use)
falls back to running cargo in the `rust:1-bookworm` Docker image, e.g. `node scripts/cargo.mjs build`.

## Conventions and gotchas
- **SQL is runtime-checked** (`sqlx::query_as::<_, T>(sql)`), not the `query!` macros, so builds
  don't need a live DB. Bind every value; never format user input into SQL. `ENTRY_SELECT` and
  `PROJECT_SUMMARY_SELECT` are base queries that get WHERE clauses appended.
- `SUM()`/`COUNT()` return `BIGINT`. Cast with `COALESCE(SUM(x),0)::BIGINT` and map to `i64`.
- Durations are stored as **integer minutes** (`time_entries.minutes`, 1–1440). The UI collects
  hours and minutes and sends `minutes`. The API also caps a user's total per day at 24h.
- Business rules live in the backend, not only the UI:
  - members can only create or edit entries on non-archived projects they are members of
  - users can't change their own email (only `PATCH /api/admin/users/{id}` can)
  - admins can't demote, deactivate or delete themselves
- Removing a project member keeps their entries. Deleting a project or user cascades to their entries.
- Emails are stored lower-cased. Normalize them before lookups and inserts.
- Errors: return `AppError`. It serializes to `{"error": "..."}` and the frontend shows `error`.
- Adding a schema change means adding a **new** file `migrations/000N_description.sql`. Never edit an applied migration.
- Next.js `rewrites` are resolved at **build time**, so `BACKEND_URL` must be set during `next build`.
- With Next 16, use `useParams()` in client pages rather than the `params` prop.
- **Data fetching:** use `useApiData(path, query)` from `src/lib/hooks.ts`. Don't use `useEffect` +
  `setState`, because the `react-hooks/set-state-in-effect` lint rule rejects it. Use `reload()` after
  mutations and `mutate(fn)` for local updates. Report pages use `usePeriod()` for day/week/month state.
- **Modals with forms:** put the form in a child component rendered inside `<Modal>`. It mounts only
  while open, so initialize its state with `useState(initial)` instead of resetting it in an effect.
- Don't silence lint rules with `eslint-disable` unless there's a real reason, and add a comment
  explaining it (see the two intentional `window.location` redirects in `lib/api.ts` and `lib/auth.tsx`).
- Keep the visual language consistent: use the components in `components/ui.tsx` (Button, Card,
  Field, Modal, Badge, StatCard...) and the `brand-*` / `ink-*` colour tokens.
