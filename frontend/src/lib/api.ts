// The session lives in an HttpOnly cookie set by the backend, so page scripts never see it.
// State-changing requests echo the readable `hw_csrf` cookie in the `X-CSRF-Token` header.

const CSRF_COOKIE = "hw_csrf";

export function readCookie(name: string): string | null {
  if (typeof document === "undefined") return null;
  for (const part of document.cookie.split(";")) {
    const [key, ...rest] = part.trim().split("=");
    if (key === name) return decodeURIComponent(rest.join("="));
  }
  return null;
}

export class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
  ) {
    super(message);
  }
}

interface RequestOptions {
  method?: "GET" | "POST" | "PUT" | "PATCH" | "DELETE";
  body?: unknown;
  form?: FormData;
  query?: Record<string, string | undefined | null>;
  /** Redirect to /login when the session is missing or expired (default true). */
  redirectOn401?: boolean;
}

export async function api<T = unknown>(path: string, opts: RequestOptions = {}): Promise<T> {
  const method = opts.method ?? "GET";
  const headers: Record<string, string> = {};
  if (method !== "GET") {
    const csrf = readCookie(CSRF_COOKIE);
    if (csrf) headers["X-CSRF-Token"] = csrf;
  }

  let body: BodyInit | undefined;
  if (opts.form) {
    body = opts.form;
  } else if (opts.body !== undefined) {
    headers["Content-Type"] = "application/json";
    body = JSON.stringify(opts.body);
  }

  let url = `/api${path}`;
  if (opts.query) {
    const params = new URLSearchParams();
    for (const [k, v] of Object.entries(opts.query)) {
      if (v) params.set(k, v);
    }
    const qs = params.toString();
    if (qs) url += `?${qs}`;
  }

  const res = await fetch(url, { method, headers, body, credentials: "same-origin" });

  if (res.status === 401 && path !== "/auth/login" && opts.redirectOn401 !== false) {
    if (typeof window !== "undefined" && !window.location.pathname.startsWith("/login")) {
      // Outside React (no router here); a full reload also clears stale session state.
      // eslint-disable-next-line @next/next/no-location-assign-relative-destination
      window.location.href = "/login";
    }
  }

  const text = await res.text();
  let data: unknown = null;
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = null;
    }
  }
  if (!res.ok) {
    const message =
      (data as { error?: string } | null)?.error ||
      (text && text.length < 300 ? text : "") ||
      res.statusText ||
      "Request failed";
    throw new ApiError(message, res.status);
  }
  return data as T;
}

export function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  return "Something went wrong";
}
