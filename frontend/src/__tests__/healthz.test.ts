import { afterEach, describe, expect, it, vi } from "vitest";
import { GET } from "@/app/healthz/route";

describe("/healthz", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("reports the build revision set by CI", async () => {
    vi.stubEnv("HOURWISE_REVISION", "0123456789ABCDEF0123456789abcdef01234567");
    const res = GET();
    expect(res.status).toBe(200);
    expect(res.headers.get("Cache-Control")).toBe("no-store");
    expect(await res.json()).toEqual({
      status: "ok",
      revision: "0123456789abcdef0123456789abcdef01234567",
    });
  });

  it("reports unknown when the revision is missing or not a commit id", async () => {
    vi.stubEnv("HOURWISE_REVISION", "");
    expect((await GET().json()).revision).toBe("unknown");
    vi.stubEnv("HOURWISE_REVISION", "<script>");
    expect((await GET().json()).revision).toBe("unknown");
  });
});
