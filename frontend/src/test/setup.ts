import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, vi } from "vitest";

afterEach(() => {
  cleanup();
  document.cookie.split(";").forEach((c) => {
    const name = c.split("=")[0]?.trim();
    if (name) document.cookie = `${name}=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/`;
  });
});

// next/navigation is only available inside the Next.js runtime.
vi.mock("next/navigation", () => {
  const router = { replace: vi.fn(), push: vi.fn(), back: vi.fn(), refresh: vi.fn() };
  return {
    useRouter: () => router,
    usePathname: () => "/",
    useParams: () => ({}),
    useSearchParams: () => new URLSearchParams(),
  };
});
