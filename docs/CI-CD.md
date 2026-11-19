# CI/CD: automatic production deploys

Every push to `main` whose CI checks **all** succeed builds immutable release images and deploys
them to Dokploy: backend first, then the frontend, each verified before the next step. Pull
requests and other branches only run the checks. Everything lives in
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) and
[`scripts/ci/dokploy-deploy.sh`](../scripts/ci/dokploy-deploy.sh). The Dokploy services themselves
are set up as in [DEPLOYMENT.md](DEPLOYMENT.md).

Nothing in this pipeline touches PostgreSQL. Dokploy only replaces the two application containers.
The database service, the database volume and the `hourwise-uploads` volume are never redeployed
or recreated. The only schema changes are the backend's own migrations, which it applies on startup
as before (see [migration guidance](DEPLOYMENT.md#5-backend-application)).

## 1. Flow

```
push to main ──► backend ─┐
                 frontend ├─► e2e ─┐
                 images ──┤        ├─► publish ──► deploy (environment: production, one at a time)
                 deploy-script ────┘   (GHCR)       │
                                                    ├─ preflight: settings, sources, Auto Deploy off,
                                                    │  Swarm settings, backend address, revision order
                                                    ├─ backend: pin image → deploy → wait → status
                                                    ├─ verify /api/health/ready serves the commit
                                                    ├─ frontend: pin image → deploy → wait → status
                                                    ├─ verify /healthz, then the backend through it
                                                    └─ on a frontend failure: put the previous
                                                       frontend back and check the site answers
```

| Job | Runs on | Does |
| --- | --- | --- |
| `backend`, `frontend`, `e2e`, `images` | every PR and push | the existing checks (fmt, clippy, PostgreSQL tests, lint, types, Vitest, Next build, Playwright, Docker builds) |
| `deploy-script` | every PR and push | tests the deploy script against a mock Dokploy API, and checks the release jobs' settings |
| `publish` | push to `main`, all checks green, `PRODUCTION_PIPELINE` = `publish` or `deploy` | builds both images and pushes them to GHCR |
| `deploy` | after `publish`, `PRODUCTION_PIPELINE` = `deploy` | deploys and verifies production |

**With `PRODUCTION_PIPELINE` unset (or any other value), nothing is published and nothing is
deployed.** `publish` is skipped, and `deploy` needs `publish`.

### Why images, not Dokploy's GitHub source

Dokploy's deploy API (`application.deploy`) takes only an application id. A GitHub-sourced app
always runs `git clone --branch main --depth 1`, so it builds whatever `main` is **when the build
starts**. Another merge during the deploy would ship a commit that CI hasn't finished checking, and
the API can't be asked for a specific commit. (Checked in Dokploy's source:
`packages/server/src/utils/providers/github.ts`.)

So CI builds the images, and Dokploy only runs them. Both Dokploy apps use the **Docker** provider,
and each deploy sets the image to an immutable reference first:

```
ghcr.io/<owner>/hourwise-backend:sha-<40-char commit>@sha256:<digest>
```

The tag names the commit, and the digest pins the exact bytes that passed CI. Dokploy passes the
string unchanged to `docker pull` and to the Swarm service.

The images are built with `HOURWISE_REVISION=<commit>`. The backend reports it in
`/api/health/ready` and the frontend in `/healthz` (`"revision": "…"`). That's how the pipeline
checks which release is **actually serving traffic**. Dokploy marks a deployment `done` a few
seconds after updating the Swarm service, before the new container is healthy, so `done` alone
proves little.

### The frontend can't take traffic without the backend

The frontend's proxy target (`BACKEND_URL`) is fixed when its image is built, from the GitHub
variable `PRODUCTION_BACKEND_URL`. A wrong value would give a frontend that answers `/healthz` but
fails every API call. Four layers stop that:

1. **Before anything deploys**, the preflight compares the host in `PRODUCTION_BACKEND_URL` (the
   exact value baked into this run's image) with the backend's actual Dokploy service name
   (`appName`).
2. **Before traffic moves**, the frontend image's health check (`frontend/docker/healthcheck.mjs`)
   only reports a new container healthy after it has reached the backend through its own `/api`
   proxy. Swarm sends traffic only to healthy tasks. With `start-first` it keeps the old frontend
   until the new one is healthy, and with `FailureAction: rollback` it reverts an update whose task
   never gets healthy. The preflight refuses to deploy if a custom Swarm health check would bypass
   this, or if Update Config isn't `start-first` + `rollback` (empty is fine: that's Dokploy's
   default).
3. **After the deploy**, the pipeline checks `/healthz` and then `/api/health/ready` **through the
   new frontend**.
4. **If the frontend deploy or those checks fail**, the rollback step sets the frontend back to the
   image it ran before this release, redeploys it, and checks that `/healthz` reports the previous
   commit and that the backend answers through it. The job still fails, so you're notified. The
   rollback step also runs when Swarm already reverted the container itself: it then corrects the
   image Dokploy has stored, so a manual "Deploy" click can't bring the bad image back.

The backend is **never** rolled back automatically, because a release may have applied a migration
that the previous backend can't run against. A backend failure stops the job before the frontend is
touched.

## 2. How each guarantee is met

| Requirement | Mechanism |
| --- | --- |
| Only green `main` deploys | `publish` `needs` every check job, and its `if:` requires `push` + `refs/heads/main` + `PRODUCTION_PIPELINE`. Any failed, skipped or cancelled check means no release. |
| PRs never deploy | Same `if:`. There are no `workflow_dispatch`, `pull_request_target` or other triggers. Fork PRs get no secrets. The `production` environment only admits `main`. |
| Exact commit | Images are built by CI from `github.sha` and deployed by digest. Deploy refuses any image that isn't `:sha-<this commit>@sha256:<digest>`. The preflight refuses commits no longer on `main`. |
| No older release over a newer one | `concurrency: production-deploy` runs one deploy at a time and never cancels a running one. A newer queued run replaces an older queued one (the older shows as *cancelled*). The preflight reads the commit production runs (from its image tag) and compares it with GitHub. Older (`behind`) is skipped with a notice and changes nothing. History that doesn't line up (`diverged`, unknown) fails without changes. Re-running an old workflow run is therefore harmless. |
| Accepted ≠ succeeded | The trigger only queues a job and returns no id. The script tags the deployment with a unique description (`hourwise-ci sha=… run=…`), finds that record and polls it until `done`/`error`/`cancelled`. It then checks `applicationStatus` and the stored image, then polls the public health endpoint until it reports the release commit 3 times in a row. |
| Backend before frontend | Sequential steps in one job. A failure stops everything after it. |
| Unexpected API answers | Every Dokploy response must be JSON of the expected shape (`application.one` an object with `sourceType`, `dockerImage`, `applicationStatus`, `appName`; `deployment.all` a list), or the step fails with a message that names it. |
| No duplicate triggers | Only the `deploy` job calls Dokploy. The preflight fails if Dokploy **Auto Deploy** is on for either app. |
| Bounded | See below. |
| Secrets | Only steps that need them get them, as environment variables. The API key goes into a private header file, never onto a command line or into logs. The tests check that the key, the GitHub token and Dokploy's app environment are never printed. |

**Time limits.** Inside the script: 10 min for Dokploy to start a deployment, 15 min for it to
finish, and 5 min for the health endpoint to serve the commit. HTTP calls have 10–30 s timeouts
and at most 5 retries (network errors, 429, 5xx).

| Step | Limit |
| --- | --- |
| checkout | 3 min |
| preflight | 5 min |
| deploy backend | 30 min |
| verify backend | 7 min |
| deploy frontend | 30 min |
| verify frontend + backend | 14 min |
| frontend rollback | 40 min |
| report | 2 min |
| **job** | **140 min** (above the 131-minute sum; checked by the `deploy-script` tests) |

## 3. Secrets and variables

**Environment `production`** (Settings → Environments → `production` → Environment secrets).
Create these **only** here, not as repository secrets. GitHub falls back to repository secrets of
the same name, which would bypass the environment's branch rule.

| Secret | Value |
| --- | --- |
| `DOKPLOY_URL` | Your Dokploy base URL, e.g. `https://app.dokploy.com` for Dokploy Cloud (`/api` is added by the script; confirm it with the read-only check in section 5) |
| `DOKPLOY_API_KEY` | API key of the dedicated deploy user (section 4, step 2) |
| `DOKPLOY_BACKEND_APP_ID` | `applicationId` of `hourwise-backend` (in its Dokploy URL) |
| `DOKPLOY_FRONTEND_APP_ID` | `applicationId` of `hourwise-frontend` |

**Variables** (Settings → Secrets and variables → Actions → Variables):

| Variable | Where | Value |
| --- | --- | --- |
| `PRODUCTION_PIPELINE` | repository | unset: checks only. `publish`: also push images. `deploy`: push and deploy |
| `PRODUCTION_BACKEND_URL` | repository (the `publish` job has no environment) | `http://<backend-app-name>:8080`, the backend's **private** Dokploy service name. It's baked into the frontend image as a **build argument only**: it isn't in the runtime image's environment, and it isn't a secret. |
| `PRODUCTION_URL` | environment `production` (or repository) | `https://<app.example.com>`, the origin only |

`GITHUB_TOKEN` is used automatically. The workflow default is `contents: read`. `publish` adds
`packages: write`. `deploy` has only `contents: read` (to compare commits) plus the environment's
secrets. No image contains a secret: the backend gets `DATABASE_URL`, `JWT_SECRET` and so on from
Dokploy at runtime, and the frontend needs none.

## 4. One-time setup (manual, in this order)

1. **GitHub environment.** Create the environment `production`. Under **Deployment branches and
   tags**, choose *Selected branches* → `main`. Add the four secrets and `PRODUCTION_URL`.
   Optionally add **Required reviewers** if you want to approve each deploy.
2. **Dokploy deploy user.** Dokploy API keys have no scopes of their own: a key has its user's
   permissions. Invite a dedicated member (e.g. `ci-deploy`) with access to **only**
   `hourwise-backend` and `hourwise-frontend`. It needs permission to **read**, **edit** (the
   pipeline changes the Docker image with `application.update`) and **deploy** them, and no admin
   rights or access to the database service. Create the API key as that user, preferably with an
   expiry date, and store it as `DOKPLOY_API_KEY`.
   - Note: with read access, the key can also read those apps' environment variables
     (`DATABASE_URL`, `JWT_SECRET`, …). Treat it as a production secret.
3. **Publish the first images.** Set `PRODUCTION_BACKEND_URL` (copy the backend name from the
   frontend's current Dokploy build argument `BACKEND_URL`) and `PRODUCTION_PIPELINE=publish`.
   Merge to `main`. The `publish` job's summary lists both image references.
4. **Registry access for Dokploy.** If the repository is private, the GHCR packages are private
   too. Check both packages are linked to the repository. If the owner is an organisation, its
   package settings must let Actions create packages. Create a GitHub **classic** personal access
   token with only `read:packages` (GHCR doesn't accept fine-grained tokens).
5. **Take a database backup** in Dokploy (`hourwise-db` → Backups), in case you need to go back.
6. **Switch both apps to the Docker provider** (DEPLOYMENT.md sections 5 and 6).
   - Image: the reference from step 3. Registry URL: `ghcr.io`. Username, plus the token from
     step 4. Save; saving doesn't deploy.
   - Then check on each app:
     - Auto Deploy is off.
     - Volumes still show `hourwise-uploads` → `/data/uploads` (backend).
     - Domains are unchanged.
     - There's no `HOURWISE_REVISION` in the environment.
     - Update Config is empty or `start-first` + `rollback`.
     - The frontend's Swarm Health Check is empty or `["CMD","node","/app/healthcheck.mjs"]`.
7. Set `PRODUCTION_PIPELINE=deploy`. Then either re-run the run from step 3 (**Re-run all jobs**;
   it redeploys that commit), or merge the next change.

## 5. Testing the workflow safely

1. **Locally, no network:** `npm run test:deploy-script`. It needs bash, curl and jq. On Windows,
   use Docker:
   `docker run --rm -v "$PWD:/r:ro" node:22-alpine sh -c "apk add -q bash curl jq && node /r/scripts/ci/test-dokploy-deploy.mjs"`.

   It covers:
   - success, ordering and redeploys
   - older, diverged and off-`main` commits
   - unpinned and wrong-commit images
   - each missing secret or variable, and bad URL values
   - a wrong backend service name
   - a GitHub-sourced app, Auto Deploy on, and wrong Swarm settings
   - a bypassing health check and a rejected key
   - transient 503s and unexpected API structures
   - failed, never-starting and stuck deployments
   - non-200 and wrong-revision health answers
   - a frontend that can't reach the backend, a broken proxy, and the automatic frontend rollback
   - static checks of the workflow: gates, secrets, permissions, concurrency, timeouts
   - that secrets are never printed
2. **Pull request:** all checks run, including `deploy-script`, and `publish`/`deploy` are skipped.
3. **`PRODUCTION_PIPELINE=publish`:** a merge publishes images without deploying.
4. **Read-only API check** of the key, URL, ids and response shapes. This only reads:

   ```bash
   read -rs KEY   # paste the key; it isn't echoed or saved in shell history
   curl -sS -H "x-api-key: $KEY" "<DOKPLOY_URL>/api/application.one?applicationId=<backend-app-id>" \
     | jq '{appName, sourceType, autoDeploy, dockerImage, applicationStatus, updateConfigSwarm, healthCheckSwarm}'
   curl -sS -H "x-api-key: $KEY" "<DOKPLOY_URL>/api/deployment.all?applicationId=<backend-app-id>" \
     | jq 'if type == "array" then .[:2] | map({deploymentId, status, description}) else "NOT A LIST" end'
   unset KEY
   ```

   Repeat for the frontend id. If either output looks different (wrapped in `result`, not a list,
   fields missing), don't enable `deploy`; the pipeline would stop at that point anyway.
5. **First real deploy:** set `deploy` at a quiet time and watch the `deploy` job and Dokploy's
   deployment log.

## 6. Recovery and rollback

- **Backend step failed:** the frontend was not touched. Swarm keeps or reverts the backend
  container if the new one never got healthy. The job summary says which step failed and why.
  Fix forward (a new commit), or roll back by hand (below).
- **Frontend step failed:** the pipeline has already tried to put the previous frontend back (job
  summary: "Frontend rolled back to …"). If that rollback itself failed, the step says why. Do it
  by hand (below).
- **Retry a failed or timed-out deploy:** Actions → the run → **Re-run failed jobs**. The
  preflight allows redeploying the same commit. If a Dokploy deployment is still running, let it
  finish, or cancel it in Dokploy first. The pipeline never cancels Dokploy deployments.
- **A run cancelled by hand** (Actions → Cancel) doesn't run the rollback step. Check Dokploy.
- **Pause automatic deploys:** set `PRODUCTION_PIPELINE=publish` (or unset it).
- **Roll back by hand to a previous release:**
  1. Pause deploys (above).
  2. Find the previous image references in that release's `publish` job summary, or in GHCR
     (package → versions, tag `sha-<commit>`).
  3. In Dokploy, set the **frontend** Docker Image to its previous reference and **Deploy**. Then
     do the same for the backend. That's the reverse of the release order: the new backend is
     meant to stay compatible with the old frontend, not the other way round.
  4. **If the release added a migration, don't roll back the backend.** An older backend refuses
     to start against a database that has a migration it doesn't know (`VersionMissing`). Fix
     forward with a new commit instead, or restore the pre-deploy database backup
     ([DEPLOYMENT.md → Backups and restore](DEPLOYMENT.md#7-backups-and-restore)), which loses
     data written since then.
  5. Once fixed, set `PRODUCTION_PIPELINE=deploy` again. The next merge is newer than the
     rolled-back release, so the preflight lets it through.
- **Pipeline unavailable** (e.g. GitHub outage): DEPLOYMENT.md section 5 has a fallback that
  builds from GitHub inside Dokploy. Switch back to the Docker provider afterwards.

## 7. Limitations

- **GHCR storage:** each release adds two image versions. Private packages count against your
  GitHub plan's Packages storage quota. Delete old versions in GHCR from time to time (keep the
  last few for rollback). No automatic cleanup is configured.
- **Live API not exercised by the tests:** procedure names, fields and Swarm behaviour come from
  Dokploy's source code. Dokploy Cloud may run a different version, so run the read-only check in
  section 5 first.
- **The frontend health gate relies on Swarm** sending traffic only to healthy tasks and waiting
  for health with `start-first`. That's standard Swarm behaviour, but it wasn't tested on a Swarm
  cluster here. The pipeline's post-deploy checks and rollback step are the backstop.
- **Dokploy Cloud runs deployments one at a time per server.** A deploy can wait in Dokploy's queue
  behind other apps on the same server (the script waits up to 10 minutes for it to start).
- **Cancelled Cloud deployments are recorded as `done`** by Dokploy. The revision check on the
  health endpoint is what catches that.
- **`[skip ci]` commits** don't run the workflow, so they aren't deployed until the next normal
  merge.
- **Local and Dokploy-built images** report `"revision": "unknown"`. Only CI images carry the
  commit.
- **Third-party actions** are pinned to major versions, not commit SHAs, as in the rest of the
  workflow.

## 8. Release log

- **2026-11-18: first publish-only release validation.** With `PRODUCTION_PIPELINE=publish`, this
  documentation-only change is merged to trigger the first GHCR publication (section 5, step 3).
  Expected result: all checks pass, `publish` pushes
  `ghcr.io/<owner>/hourwise-backend:sha-<commit>` and `ghcr.io/<owner>/hourwise-frontend:sha-<commit>`,
  and `deploy` is skipped. Dokploy and PostgreSQL are not touched. `deploy` stays disabled until the
  read-only API check (section 5, step 4) and the one-time setup (section 4) are done.
- **2026-11-20: first controlled production deployment.** With `PRODUCTION_PIPELINE=deploy`, merging
  this documentation-only change runs the full pipeline for the first time: publish, then deploy
  the backend and the frontend through the Dokploy API with digest-pinned GHCR images, replacing the
  original pre-GHCR containers. Both apps use the Docker source with Auto Deploy off; the frontend
  Update Config is start-first + rollback and both Swarm health checks are empty (the images' own
  checks apply). The database and application volumes are kept as they are. Success means both
  health endpoints report the merge commit as `revision`. The frontend's automatic rollback target
  is the `f2c9380` image saved in Dokploy, not the pre-GHCR container; see section 6 for manual
  recovery.
