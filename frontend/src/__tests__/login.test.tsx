import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useRouter } from "next/navigation";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LoginPage from "@/app/login/page";
import { ApiError } from "@/lib/api";
import { makeUser } from "@/test/fixtures";

const auth = vi.hoisted(() => ({
  user: null as unknown,
  loading: false,
  login: vi.fn(),
  logout: vi.fn(),
  setUser: vi.fn(),
  refresh: vi.fn(),
}));
vi.mock("@/lib/auth", () => ({ useAuth: () => auth }));

describe("Login page", () => {
  const router = useRouter();

  beforeEach(() => {
    auth.user = null;
    auth.loading = false;
    auth.login.mockReset();
    vi.mocked(router.replace).mockReset();
  });

  it("signs a member in and goes to the timesheet", async () => {
    auth.login.mockResolvedValue(makeUser({ role: "member" }));
    const user = userEvent.setup();
    render(<LoginPage />);
    await user.type(screen.getByLabelText("Email"), "ann@example.com");
    await user.type(screen.getByLabelText("Password"), "secret123");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(auth.login).toHaveBeenCalledWith("ann@example.com", "secret123");
    expect(router.replace).toHaveBeenCalledWith("/time");
  });

  it("sends admins to the admin interface", async () => {
    auth.login.mockResolvedValue(makeUser({ role: "admin" }));
    const user = userEvent.setup();
    render(<LoginPage />);
    await user.type(screen.getByLabelText("Email"), "boss@example.com");
    await user.type(screen.getByLabelText("Password"), "secret123");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(router.replace).toHaveBeenCalledWith("/admin");
  });

  it("shows authentication errors", async () => {
    auth.login.mockRejectedValue(new ApiError("Invalid credentials or session expired", 401));
    const user = userEvent.setup();
    render(<LoginPage />);
    await user.type(screen.getByLabelText("Email"), "ann@example.com");
    await user.type(screen.getByLabelText("Password"), "wrong");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Invalid credentials");
    expect(router.replace).not.toHaveBeenCalled();
    // The button is usable again after an error.
    expect(screen.getByRole("button", { name: "Sign in" })).toBeEnabled();
  });

  it("shows rate-limit and deactivation messages from the server", async () => {
    auth.login.mockRejectedValue(
      new ApiError("Too many failed attempts. Try again in 900 seconds.", 429),
    );
    const user = userEvent.setup();
    render(<LoginPage />);
    await user.type(screen.getByLabelText("Email"), "ann@example.com");
    await user.type(screen.getByLabelText("Password"), "x");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Too many failed attempts");
  });

  it("requires both fields", async () => {
    const user = userEvent.setup();
    render(<LoginPage />);
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(auth.login).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Email")).toBeRequired();
    expect(screen.getByLabelText("Password")).toBeRequired();
  });

  it("redirects users who are already signed in", () => {
    auth.user = makeUser({ role: "member" });
    render(<LoginPage />);
    expect(router.replace).toHaveBeenCalledWith("/time");
  });
});
