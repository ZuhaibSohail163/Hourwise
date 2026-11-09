import type * as ApiModule from "@/lib/api";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { addDays, startOfWeek } from "date-fns";
import { beforeEach, describe, expect, it, vi } from "vitest";
import TimesheetPage from "@/app/(app)/time/page";
import { api } from "@/lib/api";
import { WEEK_OPTS, toISODate } from "@/lib/time";
import type { Project, TimeEntry } from "@/lib/types";
import { makeEntry, makeProject } from "@/test/fixtures";

vi.mock("@/lib/api", async (orig) => ({
  ...(await orig<typeof ApiModule>()),
  api: vi.fn(),
}));
const apiMock = vi.mocked(api);

function serve(projects: Project[] | Error, entries: TimeEntry[] | Error | "pending") {
  apiMock.mockImplementation((path: string) => {
    if (path === "/projects") {
      return projects instanceof Error ? Promise.reject(projects) : Promise.resolve(projects);
    }
    if (path === "/time-entries") {
      if (entries === "pending") return new Promise(() => {});
      return entries instanceof Error ? Promise.reject(entries) : Promise.resolve(entries);
    }
    return Promise.reject(new Error(`unexpected ${path}`));
  });
}

const today = toISODate(new Date());
const weekStart = startOfWeek(new Date(), WEEK_OPTS);
const otherDay = toISODate(addDays(weekStart, toISODate(weekStart) === today ? 1 : 0));

describe("Timesheet page", () => {
  beforeEach(() => {
    apiMock.mockReset();
  });

  it("shows the selected day's entries, the day total and the week total", async () => {
    serve(
      [makeProject()],
      [
        makeEntry({ entry_date: today, minutes: 90, description: "Standup and planning" }),
        makeEntry({ entry_date: today, minutes: 45, description: "Code review" }),
        makeEntry({ entry_date: otherDay, minutes: 120, description: "Other day" }),
      ],
    );
    render(<TimesheetPage />);

    expect(await screen.findByText("Standup and planning")).toBeInTheDocument();
    expect(screen.getByText("Code review")).toBeInTheDocument();
    expect(screen.queryByText("Other day")).not.toBeInTheDocument();

    const total = screen.getByText("Total").parentElement as HTMLElement;
    expect(within(total).getByText("2:15")).toBeInTheDocument();
    const week = screen.getAllByText("Week").find((el) => el.tagName === "DIV")
      ?.parentElement as HTMLElement;
    expect(within(week).getByText("4:15")).toBeInTheDocument();

    // It asked for exactly the current Monday–Sunday week.
    expect(apiMock).toHaveBeenCalledWith("/time-entries", {
      query: { from: toISODate(weekStart), to: toISODate(addDays(weekStart, 6)) },
    });
  });

  it("shows the weekly grid with per-day and per-project totals", async () => {
    serve(
      [makeProject({ id: "p1", name: "Website" }), makeProject({ id: "p2", name: "Mobile" })],
      [
        makeEntry({ project_id: "p1", project_name: "Website", entry_date: today, minutes: 60 }),
        makeEntry({ project_id: "p2", project_name: "Mobile", entry_date: today, minutes: 30 }),
        makeEntry({ project_id: "p1", project_name: "Website", entry_date: otherDay, minutes: 15 }),
      ],
    );
    const user = userEvent.setup();
    render(<TimesheetPage />);
    await screen.findAllByText("Website");
    await user.click(screen.getByRole("button", { name: "Week" }));

    const table = await screen.findByRole("table");
    const websiteRow = within(table).getByText("Website").closest("tr") as HTMLElement;
    expect(within(websiteRow).getByText("1:15")).toBeInTheDocument();
    const totalRow = within(table).getAllByText("Total").at(-1)?.closest("tr") as HTMLElement;
    expect(within(totalRow).getByText("1:45")).toBeInTheDocument();
  });

  it("shows an empty state when nothing was tracked", async () => {
    serve([makeProject()], []);
    render(<TimesheetPage />);
    expect(await screen.findByText("No time tracked for this day")).toBeInTheDocument();
    expect(screen.getByText(/Track time/, { selector: "div,p" })).toBeInTheDocument();
  });

  it("tells users without projects to ask an admin", async () => {
    serve([], []);
    render(<TimesheetPage />);
    expect(await screen.findByText(/ask an administrator to add you/)).toBeInTheDocument();
  });

  it("shows a spinner while loading", () => {
    serve([makeProject()], "pending");
    const { container } = render(<TimesheetPage />);
    expect(container.querySelector(".animate-spin")).not.toBeNull();
  });

  it("shows API errors", async () => {
    serve([makeProject()], new Error("The database is temporarily unavailable."));
    render(<TimesheetPage />);
    expect(await screen.findByRole("alert")).toHaveTextContent("temporarily unavailable");
  });

  it("deletes an entry after confirmation and updates the total", async () => {
    serve(
      [makeProject()],
      [
        makeEntry({ id: "keep", entry_date: today, minutes: 30, description: "Keep me" }),
        makeEntry({ id: "drop", entry_date: today, minutes: 60, description: "Delete me" }),
      ],
    );
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();
    render(<TimesheetPage />);
    await screen.findByText("Delete me");
    const row = screen.getByText("Delete me").closest("li") as HTMLElement;
    apiMock.mockResolvedValueOnce(null);
    await user.click(within(row).getByRole("button", { name: "Delete entry" }));
    expect(apiMock).toHaveBeenCalledWith("/time-entries/drop", { method: "DELETE" });
    expect(screen.queryByText("Delete me")).not.toBeInTheDocument();
    const total = screen.getByText("Total").parentElement as HTMLElement;
    expect(within(total).getByText("0:30")).toBeInTheDocument();
  });

  it("does not delete when the user cancels", async () => {
    serve([makeProject()], [makeEntry({ id: "x", entry_date: today, description: "Stay" })]);
    vi.spyOn(window, "confirm").mockReturnValue(false);
    const user = userEvent.setup();
    render(<TimesheetPage />);
    await screen.findByText("Stay");
    const calls = apiMock.mock.calls.length;
    await user.click(screen.getByRole("button", { name: "Delete entry" }));
    expect(apiMock.mock.calls.length).toBe(calls);
    expect(screen.getByText("Stay")).toBeInTheDocument();
  });
});
