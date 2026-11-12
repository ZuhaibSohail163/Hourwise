"use client";

import { format } from "date-fns";
import { Download, ListChecks } from "lucide-react";
import { useMemo } from "react";
import { entriesToCsv } from "@/lib/csv";
import type { TimeEntry } from "@/lib/types";
import {
  type Period,
  daysIn,
  formatClock,
  formatDecimal,
  formatDuration,
  fromISODate,
  sumMinutes,
  toISODate,
} from "@/lib/time";
import { Avatar, Button, Card, CardHeader, ColorDot, EmptyState, StatCard } from "./ui";

interface Bucket {
  key: string;
  label: string;
  sublabel?: string;
  color?: string;
  minutes: number;
  count: number;
}

function groupBy(entries: TimeEntry[], keyOf: (e: TimeEntry) => Bucket): Bucket[] {
  const map = new Map<string, Bucket>();
  for (const e of entries) {
    const b = keyOf(e);
    const existing = map.get(b.key);
    if (existing) {
      existing.minutes += e.minutes;
      existing.count += 1;
    } else {
      map.set(b.key, { ...b, minutes: e.minutes, count: 1 });
    }
  }
  return [...map.values()].sort((a, b) => b.minutes - a.minutes);
}

function BreakdownList({ buckets, total }: { buckets: Bucket[]; total: number }) {
  return (
    <ul className="divide-y divide-stone-100">
      {buckets.map((b) => {
        const pct = total ? Math.round((b.minutes / total) * 100) : 0;
        return (
          <li key={b.key} className="px-5 py-3">
            <div className="flex items-center justify-between gap-3 text-sm">
              <div className="flex min-w-0 items-center gap-2">
                {b.color && <ColorDot color={b.color} />}
                <span className="truncate font-medium">{b.label}</span>
                {b.sublabel && <span className="truncate text-ink-500">{b.sublabel}</span>}
              </div>
              <div className="shrink-0 tabular-nums">
                <span className="font-semibold">{formatClock(b.minutes)}</span>
                <span className="ml-2 text-ink-500">{pct}%</span>
              </div>
            </div>
            <div className="mt-2 h-2 overflow-hidden rounded-full bg-stone-100">
              <div
                className="h-full rounded-full"
                style={{ width: `${pct}%`, backgroundColor: b.color ?? "var(--color-brand-500)" }}
              />
            </div>
          </li>
        );
      })}
    </ul>
  );
}

function exportCsv(entries: TimeEntry[], filename: string) {
  const blob = new Blob([entriesToCsv(entries)], {
    type: "text/csv;charset=utf-8",
  });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

/**
 * Totals, breakdowns and the detailed work log for a set of entries.
 * Used by the member Reports page and every admin report view.
 */
export function SummaryReport({
  entries,
  from,
  to,
  period,
  showMembers = false,
  showProjects = true,
  csvName = "hourwise-report.csv",
}: {
  entries: TimeEntry[];
  from: Date;
  to: Date;
  period: Period;
  showMembers?: boolean;
  showProjects?: boolean;
  csvName?: string;
}) {
  const total = sumMinutes(entries);

  const byProject = useMemo(
    () =>
      groupBy(entries, (e) => ({
        key: e.project_id,
        label: e.project_name,
        sublabel: e.project_client ?? undefined,
        color: e.project_color,
        minutes: 0,
        count: 0,
      })),
    [entries],
  );
  const byMember = useMemo(
    () =>
      groupBy(entries, (e) => ({
        key: e.user_id,
        label: e.user_name,
        sublabel: e.user_email,
        minutes: 0,
        count: 0,
      })),
    [entries],
  );

  const days = useMemo(() => daysIn(from, to), [from, to]);
  const perDay = useMemo(() => {
    const m = new Map<string, number>();
    for (const e of entries) m.set(e.entry_date, (m.get(e.entry_date) ?? 0) + e.minutes);
    return m;
  }, [entries]);
  const daysWorked = perDay.size;
  const maxDay = Math.max(60, ...perDay.values());

  const log = useMemo(() => {
    const groups = new Map<string, TimeEntry[]>();
    for (const e of [...entries].sort(
      (a, b) => b.entry_date.localeCompare(a.entry_date) || a.user_name.localeCompare(b.user_name),
    )) {
      const list = groups.get(e.entry_date) ?? [];
      list.push(e);
      groups.set(e.entry_date, list);
    }
    return [...groups.entries()];
  }, [entries]);

  return (
    <div className="space-y-6">
      <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        <StatCard
          label="Total time"
          value={formatDuration(total)}
          hint={`${formatDecimal(total)} hours`}
          accent
        />
        <StatCard label="Entries" value={entries.length} />
        <StatCard
          label="Days with time"
          value={daysWorked}
          hint={`of ${days.length} day${days.length === 1 ? "" : "s"}`}
        />
        <StatCard
          label="Avg per day worked"
          value={daysWorked ? formatDuration(Math.round(total / daysWorked)) : "—"}
        />
      </div>

      {period !== "day" && (
        <Card>
          <CardHeader title="Daily hours" />
          <div className="overflow-x-auto px-5 py-4">
            <div
              className="flex h-44 min-w-full items-end gap-1"
              style={{ minWidth: days.length > 14 ? days.length * 22 : undefined }}
            >
              {days.map((d) => {
                const key = toISODate(d);
                const mins = perDay.get(key) ?? 0;
                const weekend = d.getDay() === 0 || d.getDay() === 6;
                return (
                  <div
                    key={key}
                    className="group flex flex-1 flex-col items-center justify-end gap-1"
                  >
                    <span className="text-[10px] text-ink-500 tabular-nums opacity-0 group-hover:opacity-100">
                      {mins ? formatClock(mins) : ""}
                    </span>
                    <div
                      className={`w-full rounded-t ${mins ? "bg-brand-500" : "bg-stone-100"}`}
                      style={{ height: `${Math.max(mins ? 4 : 2, (mins / maxDay) * 130)}px` }}
                      title={`${format(d, "EEE d MMM")}: ${formatDuration(mins)}`}
                    />
                    <span className={`text-[10px] ${weekend ? "text-ink-300" : "text-ink-500"}`}>
                      {period === "week" ? format(d, "EEE") : format(d, "d")}
                    </span>
                  </div>
                );
              })}
            </div>
          </div>
        </Card>
      )}

      <div className={`grid gap-6 ${showMembers && showProjects ? "lg:grid-cols-2" : ""}`}>
        {showProjects && (
          <Card>
            <CardHeader
              title="By project"
              subtitle={`${byProject.length} project${byProject.length === 1 ? "" : "s"}`}
            />
            {byProject.length ? (
              <BreakdownList buckets={byProject} total={total} />
            ) : (
              <EmptyState title="No time tracked in this period" />
            )}
          </Card>
        )}
        {showMembers && (
          <Card>
            <CardHeader
              title="By team member"
              subtitle={`${byMember.length} member${byMember.length === 1 ? "" : "s"}`}
            />
            {byMember.length ? (
              <BreakdownList buckets={byMember} total={total} />
            ) : (
              <EmptyState title="No time tracked in this period" />
            )}
          </Card>
        )}
      </div>

      <Card>
        <CardHeader
          title="Work summary"
          subtitle="Everything that was done in this period, day by day"
          action={
            entries.length > 0 && (
              <Button variant="secondary" size="sm" onClick={() => exportCsv(entries, csvName)}>
                <Download className="h-4 w-4" /> Export CSV
              </Button>
            )
          }
        />
        {log.length === 0 ? (
          <EmptyState icon={<ListChecks className="h-8 w-8" />} title="Nothing logged yet">
            Time entries and their notes will show up here.
          </EmptyState>
        ) : (
          <div className="divide-y divide-stone-200">
            {log.map(([date, items]) => (
              <section key={date}>
                <div className="flex items-center justify-between bg-stone-50 px-5 py-2 text-sm">
                  <span className="font-semibold">
                    {format(fromISODate(date), "EEEE, d MMMM yyyy")}
                  </span>
                  <span className="font-semibold tabular-nums">
                    {formatClock(sumMinutes(items))}
                  </span>
                </div>
                <ul className="divide-y divide-stone-100">
                  {items.map((e) => (
                    <li key={e.id} className="flex gap-3 px-5 py-3">
                      {showMembers && <Avatar name={e.user_name} size={28} className="mt-0.5" />}
                      <div className="min-w-0 flex-1">
                        <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-sm">
                          {showMembers && <span className="font-medium">{e.user_name}</span>}
                          {showMembers && <span className="text-ink-300">·</span>}
                          <ColorDot color={e.project_color} />
                          <span className="font-medium">{e.project_name}</span>
                          {e.project_client && (
                            <span className="text-ink-500">({e.project_client})</span>
                          )}
                        </div>
                        <p className="mt-0.5 text-sm break-words whitespace-pre-wrap text-ink-700">
                          {e.description || <span className="text-ink-300 italic">No notes</span>}
                        </p>
                      </div>
                      <div className="shrink-0 text-sm font-semibold tabular-nums">
                        {formatClock(e.minutes)}
                      </div>
                    </li>
                  ))}
                </ul>
              </section>
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}
