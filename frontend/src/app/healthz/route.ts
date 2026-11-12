// Liveness probe for Docker / Dokploy: answers without calling the backend, so a backend or
// database outage doesn't get the frontend container restarted.
export const dynamic = "force-dynamic";

export function GET() {
  return Response.json({ status: "ok" }, { headers: { "Cache-Control": "no-store" } });
}
