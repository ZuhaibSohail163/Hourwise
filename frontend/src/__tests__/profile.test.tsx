import type * as ApiModule from "@/lib/api";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import ProfilePage from "@/app/(app)/profile/page";
import { ApiError, api } from "@/lib/api";
import { makeUser } from "@/test/fixtures";

vi.mock("@/lib/api", async (orig) => ({
  ...(await orig<typeof ApiModule>()),
  api: vi.fn(),
}));
const apiMock = vi.mocked(api);

const auth = vi.hoisted(() => ({
  user: null as unknown,
  loading: false,
  login: vi.fn(),
  logout: vi.fn(),
  setUser: vi.fn(),
  refresh: vi.fn(),
}));
vi.mock("@/lib/auth", () => ({ useAuth: () => auth }));

describe("Profile page", () => {
  beforeEach(() => {
    apiMock.mockReset();
    auth.setUser.mockReset();
    auth.user = makeUser({ name: "Ann Lee", job_title: "Engineer", email: "ann@example.com" });
  });

  it("shows the email as read-only (only admins can change it)", () => {
    render(<ProfilePage />);
    const email = screen.getByLabelText(/^Email/);
    expect(email).toHaveValue("ann@example.com");
    expect(email).toBeDisabled();
  });

  it("updates name and job title", async () => {
    const updated = makeUser({ name: "Ann Smith", job_title: "Lead" });
    apiMock.mockResolvedValue(updated);
    const user = userEvent.setup();
    render(<ProfilePage />);

    await user.clear(screen.getByLabelText(/^Full name/));
    await user.type(screen.getByLabelText(/^Full name/), "Ann Smith");
    await user.clear(screen.getByLabelText(/^Job title/));
    await user.type(screen.getByLabelText(/^Job title/), "Lead");
    await user.click(screen.getByRole("button", { name: /save/i }));

    expect(apiMock).toHaveBeenCalledWith("/me", {
      method: "PATCH",
      body: { name: "Ann Smith", job_title: "Lead" },
    });
    expect(auth.setUser).toHaveBeenCalledWith(updated);
    expect(await screen.findByText("Profile updated.")).toBeInTheDocument();
  });

  it("shows profile errors", async () => {
    apiMock.mockRejectedValue(new ApiError("Name must be between 1 and 100 characters", 400));
    const user = userEvent.setup();
    render(<ProfilePage />);
    await user.click(screen.getByRole("button", { name: /save/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Name must be between");
  });

  it("checks that the new passwords match before calling the API", async () => {
    const user = userEvent.setup();
    render(<ProfilePage />);
    await user.type(screen.getByLabelText("Current password"), "oldpass123");
    await user.type(screen.getByLabelText("New password"), "newpass123");
    await user.type(screen.getByLabelText("Confirm new password"), "different1");
    await user.click(screen.getByRole("button", { name: /update password/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent("New passwords do not match.");
    expect(apiMock).not.toHaveBeenCalled();
  });

  it("changes the password and clears the form", async () => {
    apiMock.mockResolvedValue(null);
    const user = userEvent.setup();
    render(<ProfilePage />);
    await user.type(screen.getByLabelText("Current password"), "oldpass123");
    await user.type(screen.getByLabelText("New password"), "newpass123");
    await user.type(screen.getByLabelText("Confirm new password"), "newpass123");
    await user.click(screen.getByRole("button", { name: /update password/i }));

    expect(apiMock).toHaveBeenCalledWith("/me/password", {
      method: "PUT",
      body: { current_password: "oldpass123", new_password: "newpass123" },
    });
    expect(await screen.findByText("Password changed.")).toBeInTheDocument();
    expect(screen.getByLabelText("Current password")).toHaveValue("");
  });

  it("reports a wrong current password", async () => {
    apiMock.mockRejectedValue(new ApiError("Current password is incorrect", 400));
    const user = userEvent.setup();
    render(<ProfilePage />);
    await user.type(screen.getByLabelText("Current password"), "nope12345");
    await user.type(screen.getByLabelText("New password"), "newpass123");
    await user.type(screen.getByLabelText("Confirm new password"), "newpass123");
    await user.click(screen.getByRole("button", { name: /update password/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Current password is incorrect");
  });

  it("rejects avatars over 2 MB without uploading", async () => {
    const user = userEvent.setup();
    const { container } = render(<ProfilePage />);
    const input = container.querySelector('input[type="file"]') as HTMLInputElement;
    const big = new File([new Uint8Array(2 * 1024 * 1024 + 1)], "big.png", { type: "image/png" });
    await user.upload(input, big);
    expect(await screen.findByRole("alert")).toHaveTextContent("2 MB or smaller");
    expect(apiMock).not.toHaveBeenCalled();
  });

  it("uploads an avatar as multipart form data", async () => {
    const updated = makeUser({ avatar_url: "/uploads/avatars/x.png" });
    apiMock.mockResolvedValue(updated);
    const user = userEvent.setup();
    const { container } = render(<ProfilePage />);
    const input = container.querySelector('input[type="file"]') as HTMLInputElement;
    await user.upload(input, new File(["png"], "me.png", { type: "image/png" }));
    expect(apiMock).toHaveBeenCalledWith(
      "/me/avatar",
      expect.objectContaining({ method: "POST", form: expect.any(FormData) }),
    );
    expect(auth.setUser).toHaveBeenCalledWith(updated);
  });
});
