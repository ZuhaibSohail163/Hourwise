import { describe, expect, it } from "vitest";
import { CSV_HEADER, csvCell, entriesToCsv } from "@/lib/csv";
import { makeEntry } from "@/test/fixtures";

describe("CSV export", () => {
  it("writes a header and one row per entry, oldest first", () => {
    const csv = entriesToCsv([
      makeEntry({ entry_date: "2026-03-02", minutes: 150, description: "Later" }),
      makeEntry({ entry_date: "2026-03-01", minutes: 45, description: "Earlier" }),
    ]);
    const lines = csv.split("\n");
    expect(lines).toHaveLength(3);
    expect(lines[0]).toBe(CSV_HEADER.map((h) => `"${h}"`).join(","));
    expect(lines[1]).toBe(
      '"2026-03-01","Ann Lee","ann@example.com","ACME","Website","0","45","0.75","Earlier"',
    );
    expect(lines[2]).toContain('"2","30","2.50","Later"');
  });

  it("hours, minutes and decimal hours add up to the stored minutes", () => {
    for (const minutes of [1, 59, 60, 61, 1439, 1440]) {
      const row = entriesToCsv([makeEntry({ minutes })]).split("\n")[1];
      const cells = row.split(",").map((c) => c.replace(/"/g, ""));
      expect(Number(cells[5]) * 60 + Number(cells[6])).toBe(minutes);
      expect(Number(cells[7])).toBeCloseTo(minutes / 60, 2);
    }
  });

  it("escapes quotes and keeps commas and newlines inside a cell", () => {
    expect(csvCell('say "hi", ok\nnext')).toBe('"say ""hi"", ok\nnext"');
  });

  it("neutralises spreadsheet formulas", () => {
    expect(csvCell('=HYPERLINK("http://evil")')).toBe(`"'=HYPERLINK(""http://evil"")"`);
    expect(csvCell("+1")).toBe(`"'+1"`);
    expect(csvCell("@SUM(A1)")).toBe(`"'@SUM(A1)"`);
    expect(csvCell("normal")).toBe('"normal"');
  });

  it("handles a missing client and an empty report", () => {
    const row = entriesToCsv([makeEntry({ project_client: null })]).split("\n")[1];
    expect(row).toContain('"ann@example.com","","Website"');
    expect(entriesToCsv([]).split("\n")).toHaveLength(1);
  });
});
