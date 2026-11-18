#!/usr/bin/env bash
# Production deploy steps for Hourwise on Dokploy, run by .github/workflows/ci.yml (jobs `publish`
# and `deploy`). See docs/CI-CD.md for the full flow.
#
#   dokploy-deploy.sh validate-backend-url          # publish: check PRODUCTION_BACKEND_URL's form
#   dokploy-deploy.sh preflight                     # may this commit be deployed? (decision=…)
#   dokploy-deploy.sh deploy <name> <app-id> <image>    # pin the image, deploy, wait for the result
#   dokploy-deploy.sh verify <name> <url> [revision]    # wait until <url> serves that revision
#   dokploy-deploy.sh rollback <app-id> <image> <app-url>   # put the previous frontend back
#
# Environment:
#   DOKPLOY_URL, DOKPLOY_API_KEY                      Dokploy API (secrets; never printed)
#   DOKPLOY_BACKEND_APP_ID, DOKPLOY_FRONTEND_APP_ID   (preflight)
#   PRODUCTION_URL, PRODUCTION_BACKEND_URL            (preflight; validate-backend-url)
#   RELEASE_SHA                                       the commit whose CI run succeeded
#   GITHUB_TOKEN, GITHUB_REPOSITORY, GITHUB_API_URL, GITHUB_RUN_ID, GITHUB_RUN_ATTEMPT
#   Limits in seconds (the workflow's step timeouts are sized from these defaults):
#   DEPLOY_QUEUE_TIMEOUT (600), DEPLOY_TIMEOUT (900), HEALTH_TIMEOUT (300), POLL_INTERVAL (10).
#   ALLOW_HTTP_URLS=1 is for the mock-API tests only.
#
# Nothing here touches PostgreSQL: Dokploy only replaces the two application containers. The
# backend applies its own (non-destructive) migrations on startup.

set -euo pipefail

QUEUE_TIMEOUT="${DEPLOY_QUEUE_TIMEOUT:-600}"
DEPLOY_TIMEOUT="${DEPLOY_TIMEOUT:-900}"
HEALTH_TIMEOUT="${HEALTH_TIMEOUT:-300}"
POLL="${POLL_INTERVAL:-10}"
GITHUB_API_URL="${GITHUB_API_URL:-https://api.github.com}"
BACKEND_URL_FORM='^http://([a-z0-9][a-z0-9-]*):([0-9]{1,5})$'
PINNED_IMAGE_FORM=':sha-[0-9a-f]{40}@sha256:[0-9a-f]{64}$'

fail() {
  echo "::error title=Production deploy::$*" >&2
  summary "**Failed:** $*"
  exit 1
}
# Progress goes to stderr: stdout of `dokploy`/`compare` is data captured by the caller.
note() { echo "$*" >&2; }
warn() { echo "::warning title=Production deploy::$*" >&2; }
summary() { if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then echo "$*" >>"$GITHUB_STEP_SUMMARY"; fi; }
output() { if [[ -n "${GITHUB_OUTPUT:-}" ]]; then echo "$1=$2" >>"$GITHUB_OUTPUT"; fi; }
require() { [[ -n "${!1:-}" ]] || fail "$1 is not set (see docs/CI-CD.md → Secrets and variables)."; }
need_https() {
  [[ "$1" =~ ^https://[A-Za-z0-9.-]+(:[0-9]+)?(/.*)?$ || ("${ALLOW_HTTP_URLS:-}" == "1" && "$1" == http://*) ]] ||
    fail "$2 must be an https:// URL."
}

command -v jq >/dev/null || fail "jq is required"
command -v curl >/dev/null || fail "curl is required"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

release_sha() {
  require RELEASE_SHA
  [[ "$RELEASE_SHA" =~ ^[0-9a-f]{40}$ ]] || fail "RELEASE_SHA must be a full 40-character commit id."
}

# ---- HTTP -------------------------------------------------------------------------------------

# The API key goes into a private header file, so it never appears in a command line or a log.
dokploy_setup() {
  require DOKPLOY_URL
  require DOKPLOY_API_KEY
  local base="${DOKPLOY_URL%/}"
  base="${base%/api}"
  need_https "$base" DOKPLOY_URL
  API="$base/api"
  (umask 077 && printf 'x-api-key: %s\n' "$DOKPLOY_API_KEY" >"$WORK/dokploy.h")
}

# http <header-file|-> <method> <url> [curl args…]: body in $WORK/body, status in $HTTP_STATUS.
# Retries network errors, 429 and 5xx up to HTTP_ATTEMPTS times; other statuses are returned.
http() {
  local hdr="$1" method="$2" url="$3"
  shift 3
  local attempt=1 max="${HTTP_ATTEMPTS:-5}" wait=2 auth=()
  if [[ "$hdr" != "-" ]]; then auth=(-H "@$hdr"); fi
  while :; do
    HTTP_STATUS="$(curl -sS -X "$method" --max-time "${HTTP_MAX_TIME:-30}" --connect-timeout 10 \
      "${auth[@]}" -o "$WORK/body" -w '%{http_code}' "$@" "$url" 2>"$WORK/curl.err")" || HTTP_STATUS="000"
    case "$HTTP_STATUS" in
      000 | 429 | 5??)
        if ((attempt >= max)); then return 0; fi
        note "  ${method} ${url##*/}: HTTP ${HTTP_STATUS}, retrying in ${wait}s ($attempt/$max)"
        sleep "$wait"
        attempt=$((attempt + 1))
        wait=$((wait * 2))
        ;;
      *) return 0 ;;
    esac
  done
}

# dokploy <GET|POST> <procedure> <json-or-query…>: prints the JSON body, fails clearly otherwise.
dokploy() {
  local method="$1" proc="$2"
  shift 2
  if [[ "$method" == GET ]]; then
    local args=(-G)
    for kv in "$@"; do args+=(--data-urlencode "$kv"); done
    http "$WORK/dokploy.h" GET "$API/$proc" "${args[@]}"
  else
    http "$WORK/dokploy.h" POST "$API/$proc" -H 'Content-Type: application/json' --data "$1"
  fi
  case "$HTTP_STATUS" in
    2??) ;;
    000) fail "Dokploy API $proc: no response ($(head -c 200 "$WORK/curl.err"))." ;;
    401 | 403) fail "Dokploy API $proc: HTTP $HTTP_STATUS. Check DOKPLOY_API_KEY and that its user has read, edit and deploy access to the application." ;;
    404) fail "Dokploy API $proc: HTTP 404. Check DOKPLOY_URL and the application ids." ;;
    *) fail "Dokploy API $proc: HTTP $HTTP_STATUS: $(head -c 300 "$WORK/body" | tr -d '\r\n')" ;;
  esac
  jq -e . "$WORK/body" >/dev/null 2>&1 ||
    fail "Dokploy API $proc returned something that is not JSON. Is DOKPLOY_URL the Dokploy address?"
  cat "$WORK/body"
}

# app_json <id>: application.one, checked for the fields this script relies on.
app_json() {
  local app
  app="$(dokploy GET application.one "applicationId=$1")"
  jq -e 'type == "object" and has("sourceType") and has("dockerImage") and has("applicationStatus") and has("appName")' \
    <<<"$app" >/dev/null ||
    fail "Dokploy API application.one returned an unexpected structure (fields: $(jq -c 'if type == "object" then keys[:12] else type end' <<<"$app")). Has the Dokploy API changed?"
  echo "$app"
}

github() {
  require GITHUB_TOKEN
  require GITHUB_REPOSITORY
  if [[ ! -f "$WORK/github.h" ]]; then
    (umask 077 && printf 'Authorization: Bearer %s\nAccept: application/vnd.github+json\n' "$GITHUB_TOKEN" >"$WORK/github.h")
  fi
  http "$WORK/github.h" GET "$GITHUB_API_URL/repos/$GITHUB_REPOSITORY/$1"
}

# compare <base> <head>: GitHub's status of head relative to base (ahead|behind|identical|diverged),
# or "missing" when either commit is unknown to the repository.
compare() {
  github "compare/$1...$2"
  case "$HTTP_STATUS" in
    200) jq -r '.status // "unexpected"' "$WORK/body" ;;
    404 | 422) echo missing ;;
    *) fail "GitHub compare: HTTP $HTTP_STATUS" ;;
  esac
}

# The release revision CI encoded in a Dokploy image reference (…:sha-<40 hex>@sha256:…).
image_revision() { sed -nE 's/.*:sha-([0-9a-f]{40})(@.*)?$/\1/p' <<<"$1"; }

# ---- validate-backend-url ---------------------------------------------------------------------

validate_backend_url() {
  require PRODUCTION_BACKEND_URL
  [[ "$PRODUCTION_BACKEND_URL" =~ $BACKEND_URL_FORM ]] ||
    fail "PRODUCTION_BACKEND_URL must look like http://<backend-app-name>:8080 (the backend's private Dokploy service name), not '$PRODUCTION_BACKEND_URL'."
}

# ---- preflight --------------------------------------------------------------------------------

# Swarm settings that make a bad container harmless: the old task keeps serving until the new one
# is healthy, and an update whose task never gets healthy is rolled back.
check_swarm() {
  local name="$1" app="$2" order failure test
  if ! jq -e 'has("updateConfigSwarm")' <<<"$app" >/dev/null; then
    warn "hourwise-$name: this Dokploy API doesn't show Swarm settings; check Update Config by hand (docs/CI-CD.md)."
    return
  fi
  # Empty = Dokploy's default, which is {Parallelism: 1, Order: start-first, FailureAction: rollback}.
  if jq -e '.updateConfigSwarm != null' <<<"$app" >/dev/null; then
    order="$(jq -r '.updateConfigSwarm.Order // ""' <<<"$app")"
    failure="$(jq -r '.updateConfigSwarm.FailureAction // ""' <<<"$app")"
    [[ "$order" == start-first && "$failure" == rollback ]] ||
      fail "hourwise-$name: Advanced → Swarm Settings → Update Config must have \"Order\": \"start-first\" and \"FailureAction\": \"rollback\", or be empty (now: '${order:-unset}', '${failure:-unset}'). See docs/DEPLOYMENT.md."
  fi
  if [[ "$name" == frontend ]]; then
    # A custom Swarm health check replaces the image's, which is what keeps a frontend that can't
    # reach the backend away from traffic.
    test="$(jq -c '.healthCheckSwarm.Test // empty' <<<"$app")"
    [[ -z "$test" || "$test" == *healthcheck.mjs* ]] ||
      fail "hourwise-frontend: its Swarm Health Check overrides the image's backend check. Clear it, or set Test to [\"CMD\",\"node\",\"/app/healthcheck.mjs\"] (docs/DEPLOYMENT.md)."
  fi
}

preflight() {
  release_sha
  require DOKPLOY_BACKEND_APP_ID
  require DOKPLOY_FRONTEND_APP_ID
  require PRODUCTION_URL
  need_https "$PRODUCTION_URL" PRODUCTION_URL
  [[ "$PRODUCTION_URL" =~ ^https?://[A-Za-z0-9.:-]+/?$ ]] ||
    fail "PRODUCTION_URL must be just the site's origin, e.g. https://app.example.com (no path)."
  output app_url "${PRODUCTION_URL%/}"
  validate_backend_url
  dokploy_setup

  # The commit must still be on main (it isn't after a force-push that dropped it).
  local on_main
  on_main="$(compare "$RELEASE_SHA" main)"
  case "$on_main" in
    ahead | identical) ;;
    *) fail "commit $RELEASE_SHA is no longer on main ($on_main); refusing to deploy it." ;;
  esac

  local decision=deploy
  summary "| Application | Deployed now | Release |"
  summary "| --- | --- | --- |"
  for pair in "backend:$DOKPLOY_BACKEND_APP_ID" "frontend:$DOKPLOY_FRONTEND_APP_ID"; do
    local name="${pair%%:*}" id="${pair#*:}" app source image auto deployed status
    app="$(app_json "$id")"
    source="$(jq -r '.sourceType // ""' <<<"$app")"
    image="$(jq -r '.dockerImage // ""' <<<"$app")"
    auto="$(jq -r '.autoDeploy // false' <<<"$app")"
    [[ "$source" == docker ]] ||
      fail "hourwise-$name uses source '$source'. Switch it to the Docker provider first (docs/CI-CD.md → One-time setup), or Dokploy would build whatever main is at that moment."
    [[ "$auto" != true ]] ||
      fail "hourwise-$name has Dokploy Auto Deploy enabled. Disable it so only this pipeline deploys."
    check_swarm "$name" "$app"
    if [[ "$name" == backend ]]; then
      # The frontend image proxies to PRODUCTION_BACKEND_URL; it must be this backend's service.
      local host app_name
      [[ "$PRODUCTION_BACKEND_URL" =~ $BACKEND_URL_FORM ]]
      host="${BASH_REMATCH[1]}"
      app_name="$(jq -r '.appName // ""' <<<"$app")"
      [[ "$host" == "$app_name" ]] ||
        fail "PRODUCTION_BACKEND_URL points to '$host', but the backend's Dokploy service name is '$app_name'. The frontend image would not reach the backend."
    fi
    output "${name}_previous" "$image"
    deployed="$(image_revision "$image")"
    if [[ -z "$deployed" ]]; then
      note "hourwise-$name: no pipeline release deployed yet (image: ${image:-none}); this will be the first."
      summary "| hourwise-$name | (none from CI) | ${RELEASE_SHA:0:12} |"
      continue
    fi
    summary "| hourwise-$name | ${deployed:0:12} | ${RELEASE_SHA:0:12} |"
    status="$(compare "$deployed" "$RELEASE_SHA")"
    case "$status" in
      ahead) note "hourwise-$name: ${deployed:0:12} → ${RELEASE_SHA:0:12}" ;;
      identical) note "hourwise-$name: ${RELEASE_SHA:0:12} is already deployed; deploying it again." ;;
      behind)
        note "hourwise-$name runs ${deployed:0:12}, which is newer than ${RELEASE_SHA:0:12}."
        decision=superseded
        ;;
      *) fail "hourwise-$name runs ${deployed:0:12}, which is not an ancestor of ${RELEASE_SHA:0:12} ($status). Resolve it manually (docs/CI-CD.md → Recovery)." ;;
    esac
  done

  if [[ "$decision" == superseded ]]; then
    echo "::notice title=Production deploy skipped::A newer release is already in production; ${RELEASE_SHA:0:12} was not deployed."
    summary "Skipped: a newer release is already deployed."
  fi
  output decision "$decision"
}

# ---- deploy -----------------------------------------------------------------------------------

# run_deployment <name> <app-id> <image> <title> <marker>: pin, trigger, wait, check.
run_deployment() {
  local name="$1" id="$2" image="$3" title="$4" marker="$5"
  dokploy_setup

  note "hourwise-$name: pinning $image"
  dokploy POST application.update "$(jq -nc --arg id "$id" --arg img "$image" '{applicationId:$id, dockerImage:$img}')" >/dev/null
  local stored
  stored="$(app_json "$id" | jq -r '.dockerImage // ""')"
  [[ "$stored" == "$image" ]] || fail "hourwise-$name: Dokploy stored image '$stored' instead of '$image'."

  note "hourwise-$name: requesting deployment"
  dokploy POST application.deploy "$(jq -nc --arg id "$id" --arg t "$title" --arg d "$marker" \
    '{applicationId:$id, title:$t, description:$d}')" >/dev/null

  # The request only queues the job; the deployment record appears when it starts.
  local start=$SECONDS started="" dep="" status="" deployment list
  while :; do
    list="$(dokploy GET deployment.all "applicationId=$id")"
    jq -e 'type == "array"' <<<"$list" >/dev/null ||
      fail "Dokploy API deployment.all returned $(jq -r type <<<"$list"), not a list. Has the Dokploy API changed? Check hourwise-$name in Dokploy."
    deployment="$(jq -c --arg m "$marker" '[.[] | objects | select(.description == $m)] | first // empty' <<<"$list")"
    if [[ -n "$deployment" ]]; then
      dep="$(jq -r '.deploymentId // "?"' <<<"$deployment")"
      status="$(jq -r '.status // ""' <<<"$deployment")"
      if [[ -z "$started" ]]; then started=$SECONDS; fi
      [[ "$status" == running ]] || break
      if ((SECONDS - started > DEPLOY_TIMEOUT)); then
        fail "hourwise-$name: deployment $dep still running after ${DEPLOY_TIMEOUT}s. It was not cancelled; check Dokploy."
      fi
    elif ((SECONDS - start > QUEUE_TIMEOUT)); then
      fail "hourwise-$name: Dokploy did not start the deployment within ${QUEUE_TIMEOUT}s. Check its deployments tab."
    fi
    sleep "$POLL"
  done

  case "$status" in
    done) ;;
    error) fail "hourwise-$name: deployment $dep failed: $(jq -r '.errorMessage // "see the Dokploy deployment log"' <<<"$deployment" | head -c 500)" ;;
    *) fail "hourwise-$name: deployment $dep ended with status '${status:-missing}'." ;;
  esac

  local app app_status
  app="$(app_json "$id")"
  app_status="$(jq -r '.applicationStatus // ""' <<<"$app")"
  [[ "$app_status" == "done" ]] || fail "hourwise-$name: deployment $dep finished but the application status is '$app_status'."
  [[ "$(jq -r '.dockerImage // ""' <<<"$app")" == "$image" ]] ||
    fail "hourwise-$name: the image changed while deploying (another deployment?)."
  note "hourwise-$name: Dokploy deployment $dep finished in $((SECONDS - start))s"
  summary "- hourwise-$name: Dokploy deployment \`$dep\` done (image \`${image##*/}\`)"
}

deploy() {
  release_sha
  local name="$1" id="$2" image="$3"
  [[ -n "$id" ]] || fail "the Dokploy application id for $name is empty."
  [[ "$(image_revision "$image")" == "$RELEASE_SHA" && "$image" =~ $PINNED_IMAGE_FORM ]] ||
    fail "image for $name is not pinned to ${RELEASE_SHA:0:12} by digest: ${image:-<empty>}"
  run_deployment "$name" "$id" "$image" "CI ${RELEASE_SHA:0:12}" \
    "hourwise-ci sha=$RELEASE_SHA run=${GITHUB_RUN_ID:-local}.${GITHUB_RUN_ATTEMPT:-1} app=$name"
}

# ---- verify -----------------------------------------------------------------------------------

# Dokploy's "done" means the new service spec was applied; Swarm may still be starting the new
# container, or roll it back if it never gets healthy. So wait until the public URL serves the
# expected revision ("any" = just healthy), several times in a row.
verify() {
  local name="$1" url="$2" expected="${3:-}"
  if [[ -z "$expected" ]]; then
    release_sha
    expected="$RELEASE_SHA"
  fi
  need_https "$url" "the $name health URL"
  local start=$SECONDS ok=0 seen="" proto=(--proto '=https')
  if [[ "${ALLOW_HTTP_URLS:-}" == "1" ]]; then proto=(); fi
  while ((ok < 3)); do
    # One attempt per poll (the loop is the retry), so the total time stays within HEALTH_TIMEOUT.
    HTTP_ATTEMPTS=1 HTTP_MAX_TIME=10 http - GET "$url" "${proto[@]}" -H 'Cache-Control: no-cache'
    seen=none
    if [[ "$HTTP_STATUS" == 200 ]]; then
      seen="$(jq -r 'if type == "object" and .status == "ok" then (.revision // "none") else "not a health response" end' \
        "$WORK/body" 2>/dev/null || echo "not JSON")"
    fi
    if [[ "$seen" == "$expected" || ("$expected" == any && "$HTTP_STATUS" == 200 && "$seen" != "not"*) ]]; then
      ok=$((ok + 1))
    else
      ok=0
    fi
    if ((ok < 3)); then
      if ((SECONDS - start > HEALTH_TIMEOUT)); then
        fail "$name health check: ${url} answered HTTP $HTTP_STATUS with revision '${seen}' after ${HEALTH_TIMEOUT}s, expected '${expected:0:12}'. Swarm may have kept or rolled back to the previous container; check the service logs."
      fi
      sleep "$POLL"
    fi
  done
  note "$name: $url is healthy and serves '${expected:0:12}' (3 consecutive checks)"
  summary "- $name: \`${url}\` serves \`${expected:0:12}\`"
}

# ---- rollback ---------------------------------------------------------------------------------

# Puts the frontend back on the image it ran before this release and checks that the site and the
# backend behind it answer again. The backend is never rolled back automatically: a release may
# have applied a migration the previous backend can't run against (docs/CI-CD.md → Recovery).
rollback() {
  local id="$1" previous="$2" app_url="$3"
  [[ -n "$id" ]] || fail "rollback: the frontend application id is empty."
  if [[ ! "$previous" =~ $PINNED_IMAGE_FORM ]]; then
    fail "rollback: the previous frontend image ('${previous:-none}') isn't a pinned pipeline image, so it can't be restored automatically. Roll back by hand (docs/CI-CD.md → Recovery)."
  fi
  local revision
  revision="$(image_revision "$previous")"
  summary "### Rolling the frontend back to \`${revision:0:12}\`"
  run_deployment frontend "$id" "$previous" "Rollback to ${revision:0:12}" \
    "hourwise-ci rollback sha=$revision run=${GITHUB_RUN_ID:-local}.${GITHUB_RUN_ATTEMPT:-1} app=frontend"
  verify frontend "$app_url/healthz" "$revision"
  verify backend "$app_url/api/health/ready" any
  echo "::warning title=Frontend rolled back::The release failed; the frontend runs ${revision:0:12} again and the site answers. The backend stays on the new release."
  summary "Frontend rolled back to \`${revision:0:12}\`; the site answers again. The backend was not rolled back."
}

case "${1:-}" in
  validate-backend-url) validate_backend_url ;;
  preflight) preflight ;;
  deploy) deploy "${2:?name}" "${3-}" "${4-}" ;;
  verify) verify "${2:?name}" "${3:?url}" "${4-}" ;;
  rollback) rollback "${2-}" "${3-}" "${4:?app url}" ;;
  *) fail "usage: $0 validate-backend-url | preflight | deploy <name> <app-id> <image> | verify <name> <url> [revision] | rollback <app-id> <image> <app-url>" ;;
esac
