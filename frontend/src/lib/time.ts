import {
  addDays,
  addMonths,
  addWeeks,
  eachDayOfInterval,
  endOfMonth,
  endOfWeek,
  format,
  isSameMonth,
  parseISO,
  startOfMonth,
  startOfWeek,
} from "date-fns";

export type Period = "day" | "week" | "month";

export const WEEK_OPTS = { weekStartsOn: 1 as const };

export function toISODate(d: Date): string {
  return format(d, "yyyy-MM-dd");
}

/** Parses a YYYY-MM-DD string as a local date. */
export function fromISODate(s: string): Date {
  return parseISO(s);
}

/** 150 -> "2:30" (Harvest style). */
export function formatClock(minutes: number): string {
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return `${h}:${m.toString().padStart(2, "0")}`;
}

/** 150 -> "2h 30m", 60 -> "1h", 0 -> "0h". */
export function formatDuration(minutes: number): string {
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h && m) return `${h}h ${m}m`;
  if (m) return `${m}m`;
  return `${h}h`;
}

/** 150 -> "2.50" */
export function formatDecimal(minutes: number): string {
  return (minutes / 60).toFixed(2);
}

export function periodRange(period: Period, anchor: Date): { from: Date; to: Date } {
  switch (period) {
    case "day":
      return { from: anchor, to: anchor };
    case "week":
      return { from: startOfWeek(anchor, WEEK_OPTS), to: endOfWeek(anchor, WEEK_OPTS) };
    case "month":
      return { from: startOfMonth(anchor), to: endOfMonth(anchor) };
  }
}

export function shiftPeriod(period: Period, anchor: Date, dir: 1 | -1): Date {
  switch (period) {
    case "day":
      return addDays(anchor, dir);
    case "week":
      return addWeeks(anchor, dir);
    case "month":
      return addMonths(anchor, dir);
  }
}

export function periodLabel(period: Period, anchor: Date): string {
  const { from, to } = periodRange(period, anchor);
  switch (period) {
    case "day":
      return format(anchor, "EEEE, d MMMM yyyy");
    case "week":
      return isSameMonth(from, to)
        ? `${format(from, "d")} – ${format(to, "d MMM yyyy")}`
        : `${format(from, "d MMM")} – ${format(to, "d MMM yyyy")}`;
    case "month":
      return format(anchor, "MMMM yyyy");
  }
}

export function daysIn(from: Date, to: Date): Date[] {
  return eachDayOfInterval({ start: from, end: to });
}

export function sumMinutes(items: { minutes: number }[]): number {
  return items.reduce((acc, e) => acc + e.minutes, 0);
}

export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}
