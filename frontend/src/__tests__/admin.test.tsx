import type * as ApiModule from "@/lib/api";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import AdminOverview from "@/app/admin/page";
import AdminProjectsPage from "@/app/admin/projects/page";
import AdminUsersPage from "@/app/admin/users/page";
import { ApiError, api } from "@/lib/api";
import type { AdminStats } from "@/lib/types";
import { makeEntry, makeProjectSummary, makeUser, makeUserWithStats } from "@/test/fixtures";

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

type Routes = Record<string, unknown>;
function serve(routes: Routes) {
  apiMock.mockImplementation((path: string) => {
    if (!(path in routes)) return Promise.reject(new Error(`unexpected ${path}`));
    const value = routes[path];
    if (value === "pending") return new Promise(() => {});
    return value instanceof Error ? Promise.reject(value) : Promise.resolve(value);
  });
}

const stats: AdminStats = {
  users: 4,
  active_users: 3,
  projects: 3,
  active_projects: 2,
  minutes_today: 90,
  minutes_this_week: 600,
  minutes_this_month: 2430,
};

beforeEach(() => {
  apiMock.mockReset();
  auth.user = makeUser({ role: "admin", name: "Grace Hopper" });
});

describe("Admin dashboard", () => {
  it("shows team statistics, active projects and hours by member", async () => {
    serve({
      "/admin/stats": stats,
      "/admin/projects": [
        makeProjectSummary({ id: "a", name: "Website", member_count: 3, total_minutes: 75 }),
        makeProjectSummary({ id: "b", name: "Archived one", is_archived: true }),
      ],
      "/admin/time-entries": [
        makeEntry({ user_id: "u1", user_name: "Ann Lee", minutes: 120 }),
        makeEntry({ user_id: "u1", user_name: "Ann Lee", minutes: 60 }),
        makeEntry({ user_id: "u2", user_name: "Bob Ray", minutes: 30 }),
      ],
    });
    render(<AdminOverview />);
    expect(await screen.findByText("Welcome back, Grace")).toBeInTheDocument();
    expect(screen.getByText("1:30")).toBeInTheDocument(); // today
    expect(screen.getByText("10:00")).toBeInTheDocument(); // this week
    expect(screen.getByText("40:30")).toBeInTheDocument(); // this month
    expect(screen.getByText("Website")).toBeInTheDocument();
    expect(screen.queryByText("Archived one")).not.toBeInTheDocument();
    // Per-member totals for the week, largest first.
    expect(screen.getByText("3:00")).toBeInTheDocument();
    expect(screen.getByText("0:30")).toBeInTheDocument();
  });

  it("shows empty states", async () => {
    serve({
      "/admin/stats": { ...stats, minutes_today: 0, minutes_this_week: 0, minutes_this_month: 0 },
      "/admin/projects": [],
      "/admin/time-entries": [],
    });
    render(<AdminOverview />);
    expect(await screen.findByText("No projects yet")).toBeInTheDocument();
    expect(screen.getByText("No time tracked this week")).toBeInTheDocument();
  });

  it("shows a loader, then errors", async () => {
    serve({ "/admin/stats": "pending", "/admin/projects": [], "/admin/time-entries": [] });
    const { container, unmount } = render(<AdminOverview />);
    expect(container.querySelector(".animate-spin")).not.toBeNull();
    unmount();

    serve({
      "/admin/stats": new ApiError("You do not have permission to do that", 403),
      "/admin/projects": [],
      "/admin/time-entries": [],
    });
    render(<AdminOverview />);
    expect(await screen.findByRole("alert")).toHaveTextContent("permission");
  });
});

describe("Admin users page", () => {
  const users = [
    makeUserWithStats({ id: "u1", name: "Ann Lee", email: "ann@example.com", total_minutes: 150 }),
    makeUserWithStats({
      id: "u2",
      name: "Bob Ray",
      email: "bob@example.com",
      role: "admin",
      is_active: false,
      total_minutes: 0,
    }),
  ];

  it("lists people with role, status and total hours", async () => {
    serve({ "/admin/users": users });
    render(<AdminUsersPage />);
    const ann = (await screen.findByText("Ann Lee")).closest("tr") as HTMLElement;
    expect(within(ann).getByText("Member")).toBeInTheDocument();
    expect(within(ann).getByText("Active")).toBeInTheDocument();
    expect(within(ann).getByText("2:30")).toBeInTheDocument();
    const bob = screen.getByText("Bob Ray").closest("tr") as HTMLElement;
    expect(within(bob).getByText("Admin")).toBeInTheDocument();
    expect(within(bob).getByText("Inactive")).toBeInTheDocument();
  });

  it("filters by name or email", async () => {
    serve({ "/admin/users": users });
    const user = userEvent.setup();
    render(<AdminUsersPage />);
    await screen.findByText("Ann Lee");
    await user.type(screen.getByPlaceholderText("Search by name or email"), "bob@");
    expect(screen.queryByText("Ann Lee")).not.toBeInTheDocument();
    expect(screen.getByText("Bob Ray")).toBeInTheDocument();
    await user.clear(screen.getByPlaceholderText("Search by name or email"));
    await user.type(screen.getByPlaceholderText("Search by name or email"), "nobody");
    expect(screen.getByText("No people found")).toBeInTheDocument();
  });

  it("creates a person and assigns projects", async () => {
    serve({
      "/admin/users": users,
      "/admin/projects": [
        makeProjectSummary({ id: "p1", name: "Website" }),
        makeProjectSummary({ id: "p2", name: "Old", is_archived: true }),
      ],
    });
    const user = userEvent.setup();
    render(<AdminUsersPage />);
    await screen.findByText("Ann Lee");
    await user.click(screen.getByRole("button", { name: /add person|add a person|new person/i }));
    const dialog = await screen.findByRole("dialog");

    await user.type(within(dialog).getByLabelText(/^Full name/), "Cara Diaz");
    await user.type(within(dialog).getByLabelText(/^Email/), "cara@example.com");
    await user.type(within(dialog).getByLabelText(/^Initial password/), "welcome123");
    await user.selectOptions(within(dialog).getByLabelText(/^Role/), "admin");
    // Archived projects are not offered.
    expect(within(dialog).queryByRole("button", { name: "Old" })).not.toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Website" }));

    apiMock.mockImplementationOnce(() => Promise.resolve(makeUser()));
    await user.click(within(dialog).getByRole("button", { name: "Add person" }));
    expect(apiMock).toHaveBeenCalledWith("/admin/users", {
      method: "POST",
      body: {
        name: "Cara Diaz",
        email: "cara@example.com",
        password: "welcome123",
        role: "admin",
        job_title: "",
        project_ids: ["p1"],
      },
    });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("shows errors from the server in the dialog", async () => {
    serve({ "/admin/users": users, "/admin/projects": [] });
    const user = userEvent.setup();
    render(<AdminUsersPage />);
    await screen.findByText("Ann Lee");
    await user.click(screen.getByRole("button", { name: /add person|add a person|new person/i }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText(/^Full name/), "Dup");
    await user.type(within(dialog).getByLabelText(/^Email/), "ann@example.com");
    await user.type(within(dialog).getByLabelText(/^Initial password/), "welcome123");
    apiMock.mockImplementationOnce(() =>
      Promise.reject(new ApiError("A user with that email already exists", 409)),
    );
    await user.click(within(dialog).getByRole("button", { name: "Add person" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("already exists");
  });
});

describe("Admin projects page", () => {
  const projects = [
    makeProjectSummary({
      id: "p1",
      name: "Website",
      client: "ACME",
      member_count: 3,
      total_minutes: 75,
    }),
    makeProjectSummary({ id: "p2", name: "Legacy", client: "Initech", is_archived: true }),
  ];

  it("lists active projects with member counts and hours", async () => {
    serve({ "/admin/projects": projects });
    render(<AdminProjectsPage />);
    const row = (await screen.findByText("Website")).closest("tr") as HTMLElement;
    expect(within(row).getByText("ACME")).toBeInTheDocument();
    expect(within(row).getByText("3")).toBeInTheDocument();
    expect(within(row).getByText("1:15")).toBeInTheDocument();
    expect(screen.queryByText("Legacy")).not.toBeInTheDocument();
  });

  it("switches between active, archived and all", async () => {
    serve({ "/admin/projects": projects });
    const user = userEvent.setup();
    render(<AdminProjectsPage />);
    await screen.findByText("Website");
    await user.click(screen.getByRole("button", { name: "Archived" }));
    expect(screen.getByText("Legacy")).toBeInTheDocument();
    expect(screen.queryByText("Website")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "All" }));
    expect(screen.getByText("Legacy")).toBeInTheDocument();
    expect(screen.getByText("Website")).toBeInTheDocument();
  });

  it("searches by client", async () => {
    serve({ "/admin/projects": projects });
    const user = userEvent.setup();
    render(<AdminProjectsPage />);
    await screen.findByText("Website");
    await user.type(screen.getByPlaceholderText("Search projects or clients"), "zzz");
    expect(screen.getByText("No projects found")).toBeInTheDocument();
  });

  it("creates a project", async () => {
    serve({ "/admin/projects": [] });
    const user = userEvent.setup();
    render(<AdminProjectsPage />);
    await screen.findByText("No projects found");
    await user.click(screen.getByRole("button", { name: /new project/i }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText(/^Project name/), "Intranet");
    await user.type(within(dialog).getByLabelText(/^Client/), "Globex");
    apiMock.mockImplementationOnce(() => Promise.resolve(makeProjectSummary({ name: "Intranet" })));
    await user.click(within(dialog).getByRole("button", { name: /create|save/i }));
    expect(apiMock).toHaveBeenCalledWith(
      "/admin/projects",
      expect.objectContaining({
        method: "POST",
        body: expect.objectContaining({ name: "Intranet", client: "Globex" }),
      }),
    );
  });

  it("shows load errors", async () => {
    serve({ "/admin/projects": new ApiError("The database is temporarily unavailable.", 503) });
    render(<AdminProjectsPage />);
    expect(await screen.findByRole("alert")).toHaveTextContent("temporarily unavailable");
  });
});
