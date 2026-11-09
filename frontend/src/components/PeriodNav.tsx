"use client";

import { ChevronLeft, ChevronRight } from "lucide-react";
import { type Period, periodLabel, shiftPeriod, toISODate, fromISODate } from "@/lib/time";
import { Segmented } from "./ui";

/** Day / Week / Month switcher with prev / next / today navigation. */
export function PeriodNav({
  period,
  anchor,
  onPeriodChange,
  onAnchorChange,
}: {
  period: Period;
  anchor: Date;
  onPeriodChange: (p: Period) => void;
  onAnchorChange: (d: Date) => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-3">
      <div className="inline-flex items-center rounded-lg border border-stone-300 bg-white shadow-sm">
        <button
          className="rounded-l-lg p-2 text-ink-700 hover:bg-stone-100"
          onClick={() => onAnchorChange(shiftPeriod(period, anchor, -1))}
          aria-label="Previous"
        >
          <ChevronLeft className="h-4 w-4" />
        </button>
        <button
          className="border-x border-stone-300 px-3 py-1.5 text-sm font-medium hover:bg-stone-100"
          onClick={() => onAnchorChange(new Date())}
        >
          Today
        </button>
        <button
          className="rounded-r-lg p-2 text-ink-700 hover:bg-stone-100"
          onClick={() => onAnchorChange(shiftPeriod(period, anchor, 1))}
          aria-label="Next"
        >
          <ChevronRight className="h-4 w-4" />
        </button>
      </div>
      <input
        type="date"
        value={toISODate(anchor)}
        onChange={(e) => e.target.value && onAnchorChange(fromISODate(e.target.value))}
        className="rounded-lg border border-stone-300 bg-white px-2 py-1.5 text-sm shadow-sm"
        aria-label="Jump to date"
      />
      <Segmented
        value={period}
        onChange={onPeriodChange}
        options={[
          { value: "day", label: "Day" },
          { value: "week", label: "Week" },
          { value: "month", label: "Month" },
        ]}
      />
      <span className="text-base font-semibold text-ink-900">{periodLabel(period, anchor)}</span>
    </div>
  );
}
