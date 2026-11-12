import { formatDecimal } from "./time";
import type { TimeEntry } from "./types";

export const CSV_HEADER = [
  "Date",
  "Member",
  "Email",
  "Client",
  "Project",
  "Hours",
  "Minutes",
  "Decimal hours",
  "Notes",
];

/**
 * Quotes a CSV cell. Text that a spreadsheet would treat as a formula (=, +, -, @, tab, CR) is
 * prefixed with an apostrophe so opening an export can't run injected formulas.
 */
export function csvCell(value: string): string {
  const safe = /^[=+\-@\t\r]/.test(value) ? `'${value}` : value;
  return `"${safe.replace(/"/g, '""')}"`;
}

/** Builds the report CSV: one row per entry, oldest first. */
export function entriesToCsv(entries: TimeEntry[]): string {
  const rows = [...entries]
    .sort((a, b) => a.entry_date.localeCompare(b.entry_date))
    .map((e) =>
      [
        e.entry_date,
        e.user_name,
        e.user_email,
        e.project_client ?? "",
        e.project_name,
        String(Math.floor(e.minutes / 60)),
        String(e.minutes % 60),
        formatDecimal(e.minutes),
        e.description,
      ]
        .map(csvCell)
        .join(","),
    );
  return [CSV_HEADER.map(csvCell).join(","), ...rows].join("\n");
}
