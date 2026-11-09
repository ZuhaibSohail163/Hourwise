import { describe, expect, it } from "vitest";
import {
  daysIn,
  formatClock,
  formatDecimal,
  formatDuration,
  fromISODate,
  initials,
  periodLabel,
  periodRange,
  shiftPeriod,
  sumMinutes,
  toISODate,
} from "@/lib/time";

describe("duration formatting", () => {
  it("formats minutes as h:mm", () => {
    expect(formatClock(0)).toBe("0:00");
    expect(formatClock(5)).toBe("0:05");
    expect(formatClock(150)).toBe("2:30");
    expect(formatClock(1440)).toBe("24:00");
    expect(formatClock(6000)).toBe("100:00");
  });

  it("formats human durations", () => {
    expect(formatDuration(0)).toBe("0h");
    expect(formatDuration(45)).toBe("45m");
    expect(formatDuration(60)).toBe("1h");
    expect(formatDuration(150)).toBe("2h 30m");
  });

  it("formats decimal hours with two places", () => {
    expect(formatDecimal(150)).toBe("2.50");
    expect(formatDecimal(20)).toBe("0.33");
    expect(formatDecimal(1)).toBe("0.02");
    expect(formatDecimal(0)).toBe("0.00");
  });

  it("sums minutes", () => {
    expect(sumMinutes([])).toBe(0);
    expect(sumMinutes([{ minutes: 30 }, { minutes: 45 }, { minutes: 1365 }])).toBe(1440);
  });
});

describe("dates", () => {
  it("round-trips ISO dates as local dates (no time zone shift)", () => {
    for (const s of ["2026-01-01", "2026-03-29", "2024-02-29", "2026-12-31"]) {
      expect(toISODate(fromISODate(s))).toBe(s);
    }
  });

  it("weeks run Monday to Sunday", () => {
    const { from, to } = periodRange("week", fromISODate("2026-11-15")); // Sunday
    expect(toISODate(from)).toBe("2026-11-09");
    expect(toISODate(to)).toBe("2026-11-15");
    const crossYear = periodRange("week", fromISODate("2027-01-01"));
    expect(toISODate(crossYear.from)).toBe("2026-12-28");
    expect(toISODate(crossYear.to)).toBe("2027-01-03");
  });

  it("months cover the whole calendar month, including leap days", () => {
    const feb = periodRange("month", fromISODate("2024-02-10"));
    expect(toISODate(feb.from)).toBe("2024-02-01");
    expect(toISODate(feb.to)).toBe("2024-02-29");
    expect(daysIn(feb.from, feb.to)).toHaveLength(29);
  });

  it("a day period is a single day", () => {
    const { from, to } = periodRange("day", fromISODate("2026-03-29"));
    expect(toISODate(from)).toBe("2026-03-29");
    expect(toISODate(to)).toBe("2026-03-29");
  });

  it("shifts periods forward and back", () => {
    const d = fromISODate("2026-01-31");
    expect(toISODate(shiftPeriod("day", d, 1))).toBe("2026-02-01");
    expect(toISODate(shiftPeriod("week", d, -1))).toBe("2026-01-24");
    expect(toISODate(shiftPeriod("month", d, 1))).toBe("2026-02-28");
  });

  it("labels periods", () => {
    expect(periodLabel("month", fromISODate("2026-11-09"))).toBe("November 2026");
    expect(periodLabel("week", fromISODate("2026-11-09"))).toBe("9 – 15 Nov 2026");
    expect(periodLabel("week", fromISODate("2026-12-30"))).toBe("28 Dec – 3 Jan 2027");
    expect(periodLabel("day", fromISODate("2026-11-09"))).toBe("Monday, 9 November 2026");
  });
});

describe("initials", () => {
  it("uses first and last name", () => {
    expect(initials("Ann Marie Lee")).toBe("AL");
    expect(initials("ann")).toBe("AN");
    expect(initials("   ")).toBe("?");
  });
});
