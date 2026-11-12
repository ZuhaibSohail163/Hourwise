import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, api, errorMessage, readCookie } from "@/lib/api";

function respond(status: number, body?: unknown) {
  return new Response(body === undefined ? null : JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

describe("api()", () => {
  const fetchMock = vi.fn();
  const realLocation = window.location;
  let location: { pathname: string; href: string };

  beforeEach(() => {
    vi.stubGlobal("fetch", fetchMock);
    location = { pathname: "/time", href: "http://localhost/time" };
    Object.defineProperty(window, "location", { configurable: true, value: location });
  });
  afterEach(() => {
    fetchMock.mockReset();
    vi.unstubAllGlobals();
    Object.defineProperty(window, "location", { configurable: true, value: realLocation });
  });

  it("GETs JSON with the session cookie and no CSRF header", async () => {
    document.cookie = "hw_csrf=tok123";
    fetchMock.mockResolvedValue(respond(200, [{ id: 1 }]));
    await expect(api("/projects")).resolves.toEqual([{ id: 1 }]);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/api/projects");
    expect(init.method).toBe("GET");
    expect(init.credentials).toBe("same-origin");
    expect(init.headers["X-CSRF-Token"]).toBeUndefined();
  });

  it("sends the CSRF token and JSON body on writes", async () => {
    document.cookie = "hw_csrf=tok123";
    fetchMock.mockResolvedValue(respond(201, { ok: true }));
    await api("/time-entries", { method: "POST", body: { minutes: 30 } });
    const [, init] = fetchMock.mock.calls[0];
    expect(init.headers["X-CSRF-Token"]).toBe("tok123");
    expect(init.headers["Content-Type"]).toBe("application/json");
    expect(init.body).toBe('{"minutes":30}');
  });

  it("never stores session tokens in localStorage", async () => {
    fetchMock.mockResolvedValue(respond(200, { user: {} }));
    await api("/auth/login", { method: "POST", body: { email: "a", password: "b" } });
    expect(window.localStorage.length).toBe(0);
  });

  it("builds query strings and drops empty values", async () => {
    fetchMock.mockResolvedValue(respond(200, []));
    await api("/time-entries", { query: { from: "2026-01-01", to: "", project_id: null } });
    expect(fetchMock.mock.calls[0][0]).toBe("/api/time-entries?from=2026-01-01");
  });

  it("raises ApiError with the server message", async () => {
    fetchMock.mockResolvedValue(
      respond(400, { error: "Total time for a day cannot exceed 24 hours" }),
    );
    const err = await api("/time-entries", { method: "POST", body: {} }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).status).toBe(400);
    expect(errorMessage(err)).toBe("Total time for a day cannot exceed 24 hours");
  });

  it("falls back to the response text for non-JSON errors", async () => {
    fetchMock.mockResolvedValue(new Response("Bad Gateway", { status: 502 }));
    await expect(api("/me")).rejects.toThrow("Bad Gateway");
  });

  it("returns null for empty 204 responses", async () => {
    fetchMock.mockResolvedValue(new Response(null, { status: 204 }));
    await expect(api("/time-entries/x", { method: "DELETE" })).resolves.toBeNull();
  });

  it("redirects to /login when the session expires", async () => {
    fetchMock.mockResolvedValue(respond(401, { error: "expired" }));
    await expect(api("/me")).rejects.toBeInstanceOf(ApiError);
    expect(location.href).toBe("/login");
  });

  it("does not redirect for failed logins or when asked not to", async () => {
    fetchMock.mockResolvedValue(respond(401, { error: "Invalid credentials" }));
    await expect(api("/auth/login", { method: "POST", body: {} })).rejects.toThrow(
      "Invalid credentials",
    );
    fetchMock.mockResolvedValue(respond(401, { error: "Invalid credentials" }));
    await expect(api("/me", { redirectOn401: false })).rejects.toBeInstanceOf(ApiError);
    expect(location.href).toBe("http://localhost/time");
  });

  it("reads cookies by name", () => {
    document.cookie = "a=1";
    document.cookie = "hw_csrf=xyz";
    expect(readCookie("hw_csrf")).toBe("xyz");
    expect(readCookie("missing")).toBeNull();
  });

  it("errorMessage handles non-Error values", () => {
    expect(errorMessage("x")).toBe("Something went wrong");
  });
});
