// Docker / Swarm health check for the frontend container (see the Dockerfile HEALTHCHECK).
//
// 1. The Next.js server answers /healthz (liveness).
// 2. Until this container has once reached the backend through its own /api proxy, it is NOT
//    healthy. Swarm only sends traffic to healthy tasks and, with the `start-first` update order,
//    keeps the old frontend running until the new one is healthy. So a new frontend that can't
//    reach the backend (e.g. a wrong BACKEND_URL baked into the image) never gets traffic and is
//    rolled back (`FailureAction: rollback`).
// 3. After that first success, only liveness counts, so a later backend or database outage
//    doesn't make Swarm restart a frontend that is working fine.
import { existsSync, writeFileSync } from "node:fs";

const MARKER = "/tmp/hourwise-backend-reached";
const PORT = process.env.PORT || "3000";

async function ok(path) {
  try {
    const res = await fetch(`http://127.0.0.1:${PORT}${path}`, {
      signal: AbortSignal.timeout(4000),
    });
    return res.ok;
  } catch {
    return false;
  }
}

if (!(await ok("/healthz"))) {
  console.error("unhealthy: /healthz did not answer 200");
  process.exit(1);
}
if (!existsSync(MARKER)) {
  if (!(await ok("/api/health"))) {
    console.error("not ready: the backend is not reachable through /api (BACKEND_URL)");
    process.exit(1);
  }
  writeFileSync(MARKER, new Date().toISOString());
}
process.exit(0);
