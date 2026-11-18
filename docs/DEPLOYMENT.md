# Deploying Hourwise with Dokploy

This guide deploys Hourwise to a server you control using [Dokploy](https://dokploy.com), a
free, self-hosted PaaS built on Docker Swarm and Traefik. Nothing here has been deployed for you.
Every value in `<angle brackets>` is a placeholder you replace.

Dokploy UI labels below come from the official docs (<https://docs.dokploy.com>, checked
2026-11). Labels marked *(check in UI)* were not confirmed in the docs. The setting is real, but
the exact wording or place may differ in your Dokploy version.

**How releases reach production:** GitHub Actions builds both images after every green merge to
`main`, pushes them to GitHub Container Registry (GHCR), and deploys them through the Dokploy API.
Dokploy only pulls and runs images; it doesn't build anything. This guide sets up the services. The
pipeline, its GitHub settings and recovery are in **[CI-CD.md](CI-CD.md)**.

## 1. Architecture

```
  GitHub Actions ── images pinned by digest ──► GHCR ◄── pulled by Dokploy on each deploy

                 Internet (HTTPS 443)
                        │
                 ┌──────▼──────┐   Let's Encrypt certificate for <app.example.com>
                 │   Traefik   │   (managed by Dokploy)
                 └──────┬──────┘
                        │ only public route
          ┌─────────────▼─────────────┐
          │ hourwise-frontend (app)    │  Next.js, port 3000, domain <app.example.com>
          │ proxies /api and /uploads  │
          └─────────────┬─────────────┘
                        │ private overlay network: http://<backend-app-name>:8080
          ┌─────────────▼─────────────┐
          │ hourwise-backend (app)     │  Rust API, port 8080, NO domain, NO published port
          │ volume /data/uploads       │
          └─────────────┬─────────────┘
                        │ private: postgres://…@<db-app-name>:5432/hourwise
          ┌─────────────▼─────────────┐
          │ PostgreSQL (database)      │  NO external port, persistent volume
          └────────────────────────────┘
```

- **One Dokploy project, `Hourwise`**, with two Applications and one PostgreSQL database.
- **Only the frontend is public.** The browser talks to one origin. The Next.js server forwards
  `/api/*` and `/uploads/*` to the backend over the private network. So there is no CORS
  configuration, cookies stay first-party (`SameSite=Strict`), and the API and database are never
  exposed to the internet.
- **Why separate Applications rather than one Compose deployment:** each service builds,
  deploys, health-checks and rolls back on its own. The database uses Dokploy's native
  PostgreSQL service with built-in backups. Applications are Swarm services, so the Swarm health
  check, update and rollback settings below apply as documented. A Compose deployment would mix
  `docker compose` and Swarm semantics, so none is provided.

## 2. Prerequisites

You need:

- **A Linux server (VPS)** with at least **2 GB RAM and 30 GB disk**. Dokploy's documented
  minimum is Ubuntu 20.04–24.04, Debian 10–12, Fedora 40 or CentOS 8/9. Images are built in
  GitHub Actions, so the server only pulls and runs them. (Building the Rust backend on the server
  instead, as in the fallback in section 5, needs about 2 GB of free RAM during the build.)
- **On Dokploy Cloud** you don't install Dokploy: connect the server under **Remote Servers**, run
  its setup, and create all three services on that server, so they share its `dokploy-network`.
- **Ports 80, 443 and 3000 free and open** in the firewall. 3000 is Dokploy's own UI. You can
  close it later (see step 9).
- **A domain name** you control, e.g. `<example.com>`, with a DNS **A record** for
  `<app.example.com>` → `<server-ip>`. Let's Encrypt needs this record to exist before you
  enable HTTPS.
- **Hourwise on GitHub**, with GitHub Actions enabled. Release images go to GHCR under the
  repository owner.
- **Optional, for backups:** an S3-compatible bucket (see section 7). Paid or free tier is your
  choice; nothing is configured for you.

Install Dokploy on the server (as root):

```bash
curl -sSL https://dokploy.com/install.sh | sh
```

To pin a version, use the `install.sh` attached to a Dokploy GitHub release instead. Then open
`http://<server-ip>:3000`, create the admin account and connect your Git provider (Settings →
Git).

## 3. Generate the secrets

Generate these on your own machine and keep them in a password manager. Never commit them.

```bash
openssl rand -hex 32   # JWT_SECRET (signs sessions; at least 32 characters)
openssl rand -hex 24   # PostgreSQL password (hex is URL-safe, so it can go in DATABASE_URL as is)
```

Also choose an **initial admin email and password** (`ADMIN_EMAIL`, `ADMIN_PASSWORD`).

## 4. Create the project and the PostgreSQL database

1. **Projects → Create Project**, name: `Hourwise`.
2. In the project: **Create Service → Database → PostgreSQL**.
   - Name: `hourwise-db`
   - Database Name: `hourwise`
   - User: `hourwise`
   - Password: the hex password from step 3
   - Docker image: `postgres:18` *(check in UI: the field offers a default image; choose 14 or
     newer)*
   - **Set the image before the first deploy.** PostgreSQL 18 images keep their data in
     `/var/lib/postgresql/18/docker` instead of the older `/var/lib/postgresql/data`, and Dokploy
     picks the volume's mount path from the image when the service is created. After creating it,
     check under **Advanced → Volumes** *(check in UI)* that the mount path is
     `/var/lib/postgresql/18/docker` or `/var/lib/postgresql`. If it is `/var/lib/postgresql/data`,
     the container refuses to start with "in 18+, these Docker images are configured to store
     database data in a format which is compatible with pg_ctlcluster".
   - **Already running PostgreSQL 17?** Don't just change the image tag on the existing service.
     PostgreSQL 18 can't open data files written by 17, and Dokploy doesn't move the volume. Instead,
     take a backup (section 7), create a **new** `postgres:18` service, restore the backup into it,
     check it, then point the backend's `DATABASE_URL` at the new service. Keep the old service
     (stopped) until you are sure.
3. **Deploy** the database.
4. Open its **Connection** section and note the **Internal Host** and the **Internal Connection
   URL**. That URL is the backend's `DATABASE_URL`. Its host is the database's internal service
   name, something like `hourwise-db-xxxxxx`.
5. **Leave `External Port (Internet)` empty.** The database must only be reachable inside the
   Dokploy network.
6. Data is stored in the database service's Docker volume, which Dokploy creates and keeps
   across redeploys.

## 5. Backend application

**Create Service → Application**, name `hourwise-backend`.

**General → Provider: Docker** *(check in UI)*:

| Field | Value |
| --- | --- |
| Docker Image | a release image from the pipeline, e.g. `ghcr.io/<owner>/hourwise-backend:sha-<commit>@sha256:<digest>` (listed in the `publish` job summary). The pipeline replaces it on every deploy. |
| Registry URL | `ghcr.io` |
| Username | a GitHub user that can read the package |
| Password | a GitHub **classic** token with only `read:packages` (GHCR doesn't accept fine-grained tokens) |

**General → Auto Deploy: off.** Only the pipeline deploys (the preflight refuses to run when this
is on).

<details>
<summary>Fallback: building from GitHub inside Dokploy (only if the pipeline is unavailable)</summary>

Provider **GitHub**, your repository, branch `main`, Build Type **Dockerfile**, Dockerfile Path
`backend/Dockerfile`, Docker Context Path `backend`. Dokploy's **Dockerfile Path** is relative to
the Git Build Path (default the repo root). Its **Docker Context Path** is relative to the repo
root; when empty, the context is the Dockerfile's folder. (Equivalent: Build Path `/backend`,
Dockerfile Path `Dockerfile`, Context empty. Never `.`, which is the repo root.) This builds the
tip of `main` at that moment, and the image reports `"revision": "unknown"`. Switch back to the
Docker provider afterwards, or the pipeline's preflight will refuse to deploy.

</details>

**Environment** (runtime variables):

```env
DATABASE_URL=<Internal Connection URL from step 4, e.g. postgresql://hourwise:<password>@<db-internal-host>:5432/hourwise>
DATABASE_MAX_CONNECTIONS=10
JWT_SECRET=<openssl rand -hex 32>
JWT_TTL_HOURS=168
COOKIE_SECURE=true
LOGIN_MAX_ATTEMPTS=5
LOGIN_MAX_ATTEMPTS_PER_IP=20
LOGIN_LOCKOUT_SECS=900
PASSWORD_HASH_CONCURRENCY=4
TRUSTED_PROXIES=<dokploy-network subnet, e.g. 10.0.1.0/24; see below>
ADMIN_EMAIL=<you@example.com>
ADMIN_PASSWORD=<initial admin password, at least 8 characters>
ADMIN_NAME=<Your Name>
RUST_LOG=hourwise_backend=info,tower_http=info
```

- `BIND_ADDR` (`0.0.0.0:8080`) and `UPLOAD_DIR` (`/data/uploads`) are already set in the image.
- Leave `CORS_ORIGIN` **unset**: the browser never calls the backend directly.
- The admin account is created on first start only if no admin exists. After your first login,
  change the password in the app and you may remove `ADMIN_PASSWORD` from the environment.
- `PASSWORD_HASH_CONCURRENCY` caps simultaneous Argon2 operations (~19 MB each). 4 suits a
  2–4 GB server. When all slots are busy for 5 seconds, a login gets `503` ("The server is busy")
  instead of using up memory.

**`TRUSTED_PROXIES` (login rate limiting).** Failed logins are limited per email *and client
address* (`LOGIN_MAX_ATTEMPTS`) and per client address (`LOGIN_MAX_ATTEMPTS_PER_IP`). The backend
only ever receives connections from the frontend container, so it learns the real client address
from `X-Forwarded-For`. Traefik adds the address it saw as the rightmost entry, and the Next.js
proxy passes the header through unchanged. The backend only believes the header when the
connection comes from an address in `TRUSTED_PROXIES`, and then it uses the rightmost entry
outside those ranges. Anything a client wrote further left is ignored.

1. On the server, print the `dokploy-network` subnet:

   ```bash
   docker network inspect dokploy-network --format '{{range .IPAM.Config}}{{.Subnet}} {{end}}'
   ```

2. Set `TRUSTED_PROXIES` to that subnet (e.g. `10.0.1.0/24`; several ranges are separated by
   commas).
3. After deploying, the backend log shows `trusting X-Forwarded-For from 10.0.1.0/24`. If it
   instead logs `ignoring X-Forwarded-For from <address>, which is not in TRUSTED_PROXIES`, the
   value doesn't cover the frontend. Every user then shares one address for rate limiting, so
   20 failed logins from anyone would block sign-in for everyone for 15 minutes. Fix the value.

Keep it safe:

- Never put public ranges or `0.0.0.0/0` in it.
- Don't publish the frontend's port outside Traefik (no entry under the app's **Ports**).
  Otherwise clients could reach Next.js directly and write the rightmost entry themselves.
- If you put another proxy or CDN in front of Traefik (e.g. Cloudflare), add its published
  address ranges too, or every user appears to come from the CDN.

**Advanced → Volumes/Mounts → Volume Mount** (avatars):

| Volume Name | Mount Path |
| --- | --- |
| `hourwise-uploads` | `/data/uploads` |

**Domains:** add **none**. The backend stays private.

**Advanced → Cluster Settings → Replicas:** `1`. The login rate limiter is in memory and avatars
are on a local volume. Running more replicas needs a shared rate-limit store and shared upload
storage, which this version doesn't have.

**Advanced → Swarm Settings → Health Check.** The image already contains a `HEALTHCHECK`. Setting
it here as well makes Dokploy and Swarm use it for zero-downtime updates. Durations are in
nanoseconds. No curl is needed: the binary probes itself.

```json
{
  "Test": ["CMD", "hourwise-backend", "healthcheck"],
  "Interval": 15000000000,
  "Timeout": 5000000000,
  "StartPeriod": 60000000000,
  "Retries": 3
}
```

**Advanced → Swarm Settings → Update Config** (start the new container first, and roll back
automatically if it never becomes healthy). Leaving it empty gives the same behaviour, because
Dokploy's default is `start-first` + `rollback`. The pipeline's preflight refuses any other
setting.

```json
{
  "Parallelism": 1,
  "Delay": 10000000000,
  "FailureAction": "rollback",
  "Order": "start-first"
}
```

> `start-first` briefly runs the old and new backend together, which gives zero-downtime updates
> **only if every migration is backward compatible**. The new container applies its migrations
> on startup (once, under a PostgreSQL advisory lock), while the **old** version keeps serving
> requests against the migrated schema until the new one is healthy. If the deploy then fails and
> is rolled back, the old version keeps running against the new schema.
>
> - **Backward compatible** (safe with `start-first`): adding a table, adding a nullable column
>   or one with a default, adding an index.
> - **Not backward compatible:** dropping or renaming a column or table, adding a `NOT NULL`
>   column without a default, or changing a column's type or meaning. Split such changes over
>   two releases ("expand, then contract"): first add the new structure and deploy code that works
>   with both, then remove the old structure in a later release. Otherwise switch Update Config to
>   `"Order": "stop-first"` for that one deploy and accept a short outage.
> - Take a database backup (**Test** button) before deploying any release with a new migration.
>
> The uploads volume is shared by both containers.

**Deploy** (normally done by the pipeline; the first time you can click Deploy once the image is
set). Dokploy pulls the image and starts it. In the logs you should see:

```
using PostgreSQL hourwise@<db-internal-host>:5432/hourwise
applied 1 database migration(s)
Created initial admin account <you@example.com>
Hourwise API listening on 0.0.0.0:8080
```

Note the backend's **internal service name** (the application's app name, e.g.
`hourwise-backend-xxxxxx`, shown in the application's General/Advanced section *(check in UI)*).
It becomes the GitHub variable `PRODUCTION_BACKEND_URL` = `http://<that name>:8080`.

**Don't set `HOURWISE_REVISION`** in the environment. The image carries the commit it was built
from, and the pipeline reads it back from `/api/health/ready`. Overriding it would make every
deploy fail its revision check.

## 6. Frontend application

**Create Service → Application**, name `hourwise-frontend`.

**General → Provider: Docker**, same fields as the backend, with the frontend image
(`ghcr.io/<owner>/hourwise-frontend:sha-<commit>@sha256:<digest>`). **Auto Deploy: off.**

**Where the backend address comes from.** Next.js fixes its `/api` and `/uploads` proxy target
when the image is **built**. CI builds the frontend with `BACKEND_URL` = the GitHub variable
`PRODUCTION_BACKEND_URL` (`http://<backend-internal-service-name>:8080`, the private name, never
a public URL). The value isn't stored in the running image's environment. Before each deploy, the
pipeline checks it against the backend's actual Dokploy service name. If you rename the backend
service, update the variable; the next release rebuilds the frontend with it.

- No Build-time Arguments in Dokploy (nothing is built there).
- Runtime environment variables: **none**. `PORT`, `HOSTNAME`, `NODE_ENV` and
  `HOURWISE_REVISION` come from the image. Don't set `HOURWISE_REVISION` here (see the backend).

**Domains → Add Domain:**

| Field | Value |
| --- | --- |
| Host | `<app.example.com>` |
| Path | `/` |
| Container Port | `3000` |
| HTTPS | on |
| Certificate | `letsencrypt` |

`traefik.me` test domains are HTTP only. Hourwise's session cookies are `Secure`, so sign-in
needs HTTPS: use your real domain.

**Advanced → Swarm Settings → Health Check:** leave it **empty**, so the image's own check
applies, or set exactly:

```json
{
  "Test": ["CMD", "node", "/app/healthcheck.mjs"],
  "Interval": 15000000000,
  "Timeout": 10000000000,
  "StartPeriod": 30000000000,
  "Retries": 3
}
```

That check (`frontend/docker/healthcheck.mjs`) only reports a **new** container healthy once it
has reached the backend through its own `/api` proxy. Swarm sends traffic only to healthy tasks,
and with `start-first` it keeps the old frontend running until the new one is healthy. So a
frontend that can't reach the backend never takes traffic and is rolled back by Swarm. After that
first success, only liveness counts, so a later backend outage doesn't get the frontend restarted.
**Any other custom health check would bypass this.** An older JSON that only fetched `/healthz`
is one example, and the pipeline's preflight refuses to deploy while one is set.

**Update Config:** the same JSON as the backend (or empty). **Replicas:** 1 is enough; the
frontend is stateless, so more replicas are fine.

**Deploy**, then open `https://<app.example.com>` and sign in with the admin account.

## 7. Backups and restore

Dokploy's database backups need an **S3-compatible destination**: Settings → Destinations
(`/dashboard/settings/destinations`). Any S3-compatible provider works. Some have free tiers
(e.g. Cloudflare R2, Backblaze B2) and others charge. **Choosing and paying for one is your
decision.** Use a bucket in a different location from the server, so a lost server doesn't take
the backups with it.

**Database backups** (database → Backups tab):

| Field | Recommended value |
| --- | --- |
| Select Destination S3 Bucket | your destination |
| Database Name | `hourwise` |
| Schedule Cron | `0 2 * * *` (daily at 02:00), or `0 */6 * * *` for every 6 hours |
| Prefix | `hourwise/db/` |
| Enabled | on |

Use **Test** to make a first backup right away. Dokploy stores `pg_dump -Fc` dumps, gzipped.

**Retention:** keep at least 14 daily backups plus a few monthly ones. If your Dokploy version has
no retention setting *(check in UI)*, use a lifecycle rule on the bucket, e.g. delete objects
under `hourwise/db/` after 35 days.

**Uploads (avatars):** Dokploy **Volume Backups** support named volumes such as
`hourwise-uploads`. Give it a schedule, the same destination and prefix `hourwise/uploads/`.
Avatars are easy to re-upload, so weekly is enough.

**Encrypt and restrict.** Backups contain names, emails, password hashes and time logs. Use a
private bucket, a key that can only write to that bucket, and provider-side encryption.

**Verify backups monthly** without touching production:

```bash
# On any machine with Docker, using a downloaded backup file. Nothing here touches production:
# the container is throwaway (--rm) and keeps its data only until it stops.
gunzip -k <backup>.sql.gz          # if it is gzipped; the result is the pg_dump archive
docker run -d --rm --name hw-restore-test -e POSTGRES_PASSWORD=test postgres:18
# Wait until PostgreSQL accepts connections (the first start takes a few seconds).
# The image briefly starts a temporary server during initialisation, so require
# several successful checks in a row before continuing.
ok=0
for i in $(seq 1 60); do
  if docker exec hw-restore-test pg_isready -U postgres -d postgres -q; then
    ok=$((ok + 1)); [ "$ok" -ge 3 ] && break
  else
    ok=0
  fi
  sleep 1
done
[ "$ok" -ge 3 ] && echo "ready" || echo "PostgreSQL did not become ready: check 'docker logs hw-restore-test' and stop here"
docker cp <backup> hw-restore-test:/tmp/backup.dump
docker exec hw-restore-test pg_restore -U postgres -d postgres --no-owner --exit-on-error /tmp/backup.dump
docker exec hw-restore-test psql -U postgres -c "SELECT COUNT(*) FROM users" -c "SELECT COUNT(*) FROM time_entries"
docker stop hw-restore-test
```

Use a `postgres` image whose major version is the same as or newer than your database's
(`pg_restore` can't read a dump from a newer server).

Locally, `npm run backup` does the same automatically for the development database.

**Restore in Dokploy** (database → Backups → **Restore**):

1. **Restore into a new database first.** Create a second PostgreSQL service (e.g.
   `hourwise-db-restore`), restore the chosen backup into it (Select Source S3 Bucket → Search for
   Backup File → Database Name → Restore Database), and check it with
   `hourwise-backend verify` (backend container → Terminal, with `DATABASE_URL` pointing at it).
2. **Switch over:** point the backend's `DATABASE_URL` at the restored database and redeploy the
   backend. The old database stays untouched, so you can switch back.
3. Restoring over the live database also works, but overwrites it. Do it only with a fresh
   backup of the current state and the backend stopped.

**Recovery considerations:**

- With daily backups you can lose up to 24 hours of entries. Shorten the schedule if that
  matters.
- Also back up Dokploy itself (Web Server → Backups). That keeps your project configuration and
  environment variables.
- Keep `JWT_SECRET` and the database password in your password manager. A restore on a new
  server needs them. A new `JWT_SECRET` only signs everyone out.

## 8. Deployment order, automatic deploys and rollbacks

**Order the first time:**

1. PostgreSQL
2. Backend, then wait until it is healthy
3. Frontend

After that, each one can be redeployed on its own.

**Automatic deploys: use the GitHub Actions pipeline, not Dokploy's Auto Deploy.** See
**[CI-CD.md](CI-CD.md)**. After every green merge to `main`, CI builds both images, pins them by
digest and deploys them through the Dokploy API, backend first, verifying each step. Keep **Auto
Deploy off** on both apps, so nothing else deploys. The pipeline refuses to run while it is on.

**Rollbacks:**

- **Automatic (Swarm):** with `"FailureAction": "rollback"` (or Dokploy's default), a new
  container that never becomes healthy is rolled back by Swarm. For the frontend, "healthy"
  includes reaching the backend (section 6).
- **Automatic (pipeline, frontend only):** if the frontend deploy or its checks fail, the pipeline
  puts the previous frontend image back, redeploys it and checks the site answers again
  ([CI-CD.md → Recovery](CI-CD.md#6-recovery-and-rollback)).
- **Manual:** set the previous image reference (from GHCR or an earlier `publish` summary) in
  Dokploy and deploy. The steps are in CI-CD.md.
- **Database migrations are forward-only.** An older backend refuses to start against a database
  migrated by a newer one ("has migration N applied, which this build doesn't know"). That's why
  the backend is never rolled back automatically. To roll back across a schema change, restore
  the backup taken before the deploy (section 7). Take one (**Test** button) before merging a
  release with a new migration.

## 9. Production checklist

- [ ] DNS A record for `<app.example.com>` points to the server. HTTPS certificate issued.
- [ ] `JWT_SECRET` is random, at least 32 characters, and stored in a password manager.
- [ ] The database password is random and URL-safe. **External Port is empty.**
- [ ] The backend has **no domain** and no published ports. The frontend is the only public
      service.
- [ ] `COOKIE_SECURE=true`. `CORS_ORIGIN` is unset.
- [ ] `TRUSTED_PROXIES` = the `dokploy-network` subnet, and the backend log says `trusting
      X-Forwarded-For from …` (not `ignoring X-Forwarded-For`). The frontend has no published Ports.
- [ ] Both apps use the **Docker** provider (GHCR, read-only token) with **Auto Deploy off**,
      and neither sets `HOURWISE_REVISION`.
- [ ] GitHub variable `PRODUCTION_BACKEND_URL` = `http://<backend-internal-service-name>:8080`.
- [ ] `hourwise-uploads` volume mounted at `/data/uploads`.
- [ ] Update Config `start-first` + `rollback` (or empty) on both apps. Frontend Swarm Health
      Check empty or `["CMD","node","/app/healthcheck.mjs"]`. Replicas: backend 1.
- [ ] First admin signed in and changed the password. `ADMIN_PASSWORD` removed from the
      environment.
- [ ] Database backups scheduled to an off-server bucket, and one test backup restored
      successfully.
- [ ] Dokploy's own backup configured. Port 3000 closed once the Dokploy UI has its own HTTPS
      domain (Dokploy docs: `docker service update --publish-rm "published=3000,target=3000,mode=host" dokploy`).
- [ ] Server: automatic security updates, SSH keys only, firewall allowing only 22/80/443.

## 10. Troubleshooting

| Symptom | Likely cause and fix |
| --- | --- |
| Backend log: `DATABASE_URL must be set` / `must be a PostgreSQL URL` | Set `DATABASE_URL` to the database's Internal Connection URL (`postgres://` or `postgresql://`). |
| Backend log: `database not reachable (attempt n/12)`, then exit | Wrong internal host or password, or the database isn't deployed. Compare with the database's Connection section. A password with `@ : / #` must be percent-encoded. |
| Backend log: `migration N was modified after it was applied` | A migration file was edited. Restore the original file and add a new migration instead. |
| Backend log: `has migration N applied, which this build doesn't know` | You deployed an older backend against a newer schema. Deploy the newer version, or restore a backup. |
| Site shows `502 Bad Gateway` for `/api/...` | The backend isn't healthy, or the frontend image was built with the wrong backend address. Fix the GitHub variable `PRODUCTION_BACKEND_URL` and release again: the value is baked in at build time. (The preflight and the frontend health check normally catch this before traffic moves.) |
| Deploy fails with `pull access denied` / `manifest unknown` | Dokploy can't read the GHCR image. Check the provider's Registry URL `ghcr.io`, username and `read:packages` token, and that the package is linked to the repository. |
| Pipeline preflight fails | Its message names the setting: Docker provider, Auto Deploy, Update Config, frontend Health Check, `PRODUCTION_BACKEND_URL` vs the backend's service name, or a missing secret. See CI-CD.md. |
| Login "works" but you're immediately signed out | The site isn't served over HTTPS, so the browser drops the `Secure` cookies. Enable HTTPS on the domain. |
| `429 Too many failed attempts` | The login lockout for that email from that address, or for that address overall. Wait `LOGIN_LOCKOUT_SECS`, or restart the backend. If **everyone** gets it at once, `TRUSTED_PROXIES` is wrong (the log says `ignoring X-Forwarded-For`): see section 5. |
| `503 The server is busy` on login | All `PASSWORD_HASH_CONCURRENCY` slots were busy for 5 s (a login burst or an attack). It recovers by itself. Raise the value only if the server has spare memory and CPU. |
| Deploy succeeded but the app isn't running | An invalid mount or health check. Check the application's Swarm/Logs section. The Dokploy docs note Swarm silently refuses invalid mounts. |
| Avatars disappear after a redeploy | The `/data/uploads` volume mount is missing. |
