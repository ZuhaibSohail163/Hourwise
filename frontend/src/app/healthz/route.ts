// Liveness probe for Docker / Dokploy: answers without calling the backend, so a backend or
// database outage doesn't get the frontend container restarted.
// `revision` is the git commit of this build (HOURWISE_REVISION, set by the CI image build). The
// deploy pipeline reads it to confirm which release is serving traffic (docs/CI-CD.md).
export const dynamic = "force-dynamic";

function revision(): string {
  const value = (process.env.HOURWISE_REVISION ?? "").trim().toLowerCase();
  return /^[0-9a-f]{7,64}$/.test(value) ? value : "unknown";
}

export function GET() {
  return Response.json(
    { status: "ok", revision: revision() },
    { headers: { "Cache-Control": "no-store" } },
  );
}
