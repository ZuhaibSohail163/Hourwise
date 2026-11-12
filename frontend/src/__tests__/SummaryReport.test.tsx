import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SummaryReport } from "@/components/SummaryReport";
import { fromISODate } from "@/lib/time";
import { makeEntry } from "@/test/fixtures";

const week = { from: fromISODate("2026-03-30"), to: fromISODate("2026-04-05") };

const entries = [
  makeEntry({
    entry_date: "2026-03-30",
    minutes: 90,
    project_id: "p1",
    project_name: "Website",
    description: "Monday work",
  }),
  makeEntry({
    entry_date: "2026-03-31",
    minutes: 45,
    project_id: "p2",
    project_name: "Mobile",
    description: "Tuesday work",
  }),
  makeEntry({
    entry_date: "2026-03-31",
    minutes: 15,
    project_id: "p1",
    project_name: "Website",
    user_id: "u2",
    user_name: "Bob Ray",
    user_email: "bob@example.com",
    description: "Bob's fix",
  }),
];

describe("SummaryReport", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("shows totals for the period", () => {
    render(<SummaryReport entries={entries} {...week} period="week" showMembers />);
    expect(screen.getByText("2h 30m")).toBeInTheDocument(); // total time
    expect(screen.getByText("2.50 hours")).toBeInTheDocument();
    expect(screen.getByText("of 7 days")).toBeInTheDocument();
    expect(screen.getByText("1h 15m")).toBeInTheDocument(); // average per day worked
  });

  it("breaks time down by project and member, and lists the work log", () => {
    render(<SummaryReport entries={entries} {...week} period="week" showMembers />);
    expect(screen.getByText("2 projects")).toBeInTheDocument();
    expect(screen.getByText("2 members")).toBeInTheDocument();
    expect(screen.getByText("Monday work")).toBeInTheDocument();
    expect(screen.getByText("Bob's fix")).toBeInTheDocument();
    // Website = 90 + 15 minutes = 70% of 150.
    expect(screen.getAllByText("70%").length).toBeGreaterThan(0);
  });

  it("shows empty states when nothing was logged", () => {
    render(<SummaryReport entries={[]} {...week} period="week" />);
    expect(screen.getByText("Nothing logged yet")).toBeInTheDocument();
    expect(screen.getByText("No time tracked in this period")).toBeInTheDocument();
  });

  it("exports the entries as CSV", async () => {
    let blob: Blob | undefined;
    vi.stubGlobal(
      "URL",
      Object.assign(Object.create(URL), {
        createObjectURL: (b: Blob) => {
          blob = b;
          return "blob:test";
        },
        revokeObjectURL: vi.fn(),
      }),
    );
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
    const user = userEvent.setup();
    render(<SummaryReport entries={entries} {...week} period="week" csvName="report.csv" />);
    await user.click(screen.getByRole("button", { name: /export csv/i }));

    expect(click).toHaveBeenCalled();
    expect(blob?.type).toBe("text/csv;charset=utf-8");
    const text = await new Promise<string>((resolve) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.readAsText(blob as Blob);
    });
    const lines = text.split("\n");
    expect(lines).toHaveLength(4);
    expect(lines[0]).toContain('"Date","Member"');
    expect(lines[1]).toContain('"2026-03-30","Ann Lee"');
    expect(lines[1]).toContain('"1","30","1.50","Monday work"');
    expect(lines[3]).toContain('"Bob\'s fix"');
  });
});
