"use client";

import clsx from "clsx";
import { addDays, addWeeks, endOfWeek, format, isToday, startOfWeek } from "date-fns";
import { CalendarDays, ChevronLeft, ChevronRight, Pencil, Plus, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import { EntryModal } from "@/components/EntryModal";
import {
  Alert,
  Button,
  Card,
  ColorDot,
  EmptyState,
  PageHeader,
  Segmented,
  Spinner,
} from "@/components/ui";
import { api, errorMessage } from "@/lib/api";
import { useApiData } from "@/lib/hooks";
import type { Project, TimeEntry } from "@/lib/types";
import { WEEK_OPTS, formatClock, fromISODate, sumMinutes, toISODate } from "@/lib/time";

type View = "day" | "week";

export default function TimesheetPage() {
  const [view, setView] = useState<View>("day");
  const [selected, setSelected] = useState<Date>(() => new Date());
  const [actionError, setActionError] = useState<string | null>(null);
  const [modalOpen, setModalOpen] = useState(false);
  const [editing, setEditing] = useState<TimeEntry | null>(null);

  // Keyed by the ISO week start so clicking days within a week doesn't refetch.
  const weekKey = toISODate(startOfWeek(selected, WEEK_OPTS));
  const weekStart = useMemo(() => fromISODate(weekKey), [weekKey]);
  const weekEnd = useMemo(() => endOfWeek(weekStart, WEEK_OPTS), [weekStart]);
  const weekDays = useMemo(
    () => Array.from({ length: 7 }, (_, i) => addDays(weekStart, i)),
    [weekStart],
  );
  const selectedKey = toISODate(selected);

  const projectsQuery = useApiData<Project[]>("/projects");
  const entriesQuery = useApiData<TimeEntry[]>("/time-entries", {
    from: weekKey,
    to: toISODate(weekEnd),
  });
  const { loading, mutate: mutateEntries } = entriesQuery;
  const projects = useMemo(() => projectsQuery.data ?? [], [projectsQuery.data]);
  const entries = useMemo(() => entriesQuery.data ?? [], [entriesQuery.data]);
  const error = actionError ?? entriesQuery.error ?? projectsQuery.error ?? null;

  const byDay = useMemo(() => {
    const m = new Map<string, TimeEntry[]>();
    for (const e of entries) {
      const list = m.get(e.entry_date) ?? [];
      list.push(e);
      m.set(e.entry_date, list);
    }
    return m;
  }, [entries]);

  const dayEntries = [...(byDay.get(selectedKey) ?? [])].sort((a, b) =>
    a.created_at.localeCompare(b.created_at),
  );
  const weekTotal = sumMinutes(entries);
  const activeProjects = projects.filter((p) => !p.is_archived);

  function openNew() {
    setEditing(null);
    setModalOpen(true);
  }

  function openEdit(e: TimeEntry) {
    setEditing(e);
    setModalOpen(true);
  }

  async function remove(e: TimeEntry) {
    if (!confirm(`Delete ${formatClock(e.minutes)} on ${e.project_name}?`)) return;
    try {
      await api(`/time-entries/${e.id}`, { method: "DELETE" });
      mutateEntries((prev) => prev.filter((x) => x.id !== e.id));
      setActionError(null);
    } catch (err) {
      setActionError(errorMessage(err));
    }
  }

  function onSaved(saved: TimeEntry) {
    // If the entry moved outside the visible week, jump to its date.
    const d = fromISODate(saved.entry_date);
    if (d < weekStart || d > weekEnd) {
      setSelected(d);
    } else {
      mutateEntries((prev) => [...prev.filter((x) => x.id !== saved.id), saved]);
      if (view === "day") setSelected(d);
    }
  }

  return (
    <>
      <PageHeader
        title={
          view === "day"
            ? format(selected, "EEEE, d MMMM")
            : `Week of ${format(weekStart, "d MMMM")}`
        }
        subtitle={
          isToday(selected) && view === "day" ? (
            <span className="font-medium text-brand-600">Today</span>
          ) : (
            format(selected, "yyyy")
          )
        }
        action={
          <>
            <div className="inline-flex items-center rounded-lg border border-stone-300 bg-white shadow-sm">
              <button
                className="rounded-l-lg p-2 hover:bg-stone-100"
                onClick={() =>
                  setSelected(view === "day" ? addDays(selected, -1) : addWeeks(selected, -1))
                }
                aria-label="Previous"
              >
                <ChevronLeft className="h-4 w-4" />
              </button>
              <button
                className="border-x border-stone-300 px-3 py-1.5 text-sm font-medium hover:bg-stone-100"
                onClick={() => setSelected(new Date())}
              >
                Today
              </button>
              <button
                className="rounded-r-lg p-2 hover:bg-stone-100"
                onClick={() =>
                  setSelected(view === "day" ? addDays(selected, 1) : addWeeks(selected, 1))
                }
                aria-label="Next"
              >
                <ChevronRight className="h-4 w-4" />
              </button>
            </div>
            <label className="relative inline-flex items-center">
              <CalendarDays className="pointer-events-none absolute left-2.5 h-4 w-4 text-ink-500" />
              <input
                type="date"
                value={selectedKey}
                onChange={(e) => e.target.value && setSelected(fromISODate(e.target.value))}
                className="rounded-lg border border-stone-300 bg-white py-1.5 pr-2 pl-8 text-sm shadow-sm"
                aria-label="Pick a date"
              />
            </label>
            <Segmented
              value={view}
              onChange={setView}
              options={[
                { value: "day", label: "Day" },
                { value: "week", label: "Week" },
              ]}
            />
          </>
        }
      />

      {error && (
        <div className="mb-4">
          <Alert>{error}</Alert>
        </div>
      )}

      {view === "day" ? (
        <div className="flex flex-col gap-6 md:flex-row md:items-start">
          <Button
            variant="success"
            size="lg"
            onClick={openNew}
            className="h-20 w-full flex-col gap-1 md:w-24"
            title="Track time"
          >
            <Plus className="h-7 w-7" />
            <span className="text-xs font-semibold">Track time</span>
          </Button>

          <Card className="flex-1 overflow-hidden">
            {/* Week strip */}
            <div className="grid grid-cols-8 border-b border-stone-200 text-center text-sm">
              {weekDays.map((d) => {
                const key = toISODate(d);
                const mins = sumMinutes(byDay.get(key) ?? []);
                const active = key === selectedKey;
                return (
                  <button
                    key={key}
                    onClick={() => setSelected(d)}
                    className={clsx(
                      "border-b-2 px-1 py-3 transition-colors hover:bg-stone-50",
                      active ? "border-brand-500" : "border-transparent",
                    )}
                  >
                    <div
                      className={clsx(
                        "font-medium",
                        active ? "text-ink-900" : "text-ink-500",
                        isToday(d) && "text-brand-600",
                      )}
                    >
                      {format(d, "EEE")}
                    </div>
                    <div className="mt-0.5 text-ink-500 tabular-nums">{formatClock(mins)}</div>
                  </button>
                );
              })}
              <div className="px-1 py-3">
                <div className="font-medium text-ink-500">Week</div>
                <div className="mt-0.5 font-semibold tabular-nums">{formatClock(weekTotal)}</div>
              </div>
            </div>

            {loading ? (
              <div className="flex justify-center py-12">
                <Spinner />
              </div>
            ) : dayEntries.length === 0 ? (
              <EmptyState
                icon={<CalendarDays className="h-9 w-9" />}
                title="No time tracked for this day"
              >
                {activeProjects.length
                  ? "Click “Track time” to log what you worked on."
                  : "You aren’t on any active projects yet — ask an administrator to add you."}
              </EmptyState>
            ) : (
              <ul className="divide-y divide-stone-100">
                {dayEntries.map((e) => (
                  <li key={e.id} className="group flex items-start gap-4 px-5 py-4">
                    <ColorDot color={e.project_color} className="mt-1.5" />
                    <div className="min-w-0 flex-1">
                      <div className="text-sm">
                        <span className="font-semibold">{e.project_name}</span>
                        {e.project_client && (
                          <span className="text-ink-500"> ({e.project_client})</span>
                        )}
                      </div>
                      <p className="mt-0.5 text-sm break-words whitespace-pre-wrap text-ink-700">
                        {e.description || <span className="text-ink-300 italic">No notes</span>}
                      </p>
                    </div>
                    <div className="text-lg font-semibold tabular-nums">
                      {formatClock(e.minutes)}
                    </div>
                    <div className="flex gap-1 opacity-100 md:opacity-0 md:group-hover:opacity-100">
                      <button
                        onClick={() => openEdit(e)}
                        className="rounded-md border border-stone-200 p-1.5 text-ink-500 hover:bg-stone-100 hover:text-ink-900"
                        aria-label="Edit entry"
                      >
                        <Pencil className="h-4 w-4" />
                      </button>
                      <button
                        onClick={() => remove(e)}
                        className="rounded-md border border-stone-200 p-1.5 text-ink-500 hover:bg-red-50 hover:text-red-600"
                        aria-label="Delete entry"
                      >
                        <Trash2 className="h-4 w-4" />
                      </button>
                    </div>
                  </li>
                ))}
                <li className="flex items-center justify-end gap-4 bg-stone-50 px-5 py-3 text-sm">
                  <span className="text-ink-500">Total</span>
                  <span className="text-lg font-semibold tabular-nums">
                    {formatClock(sumMinutes(dayEntries))}
                  </span>
                  <span className="w-[68px]" />
                </li>
              </ul>
            )}
          </Card>
        </div>
      ) : (
        <WeekGrid
          days={weekDays}
          entries={entries}
          loading={loading}
          onSelectDay={(d) => {
            setSelected(d);
            setView("day");
          }}
          onAdd={openNew}
        />
      )}

      <EntryModal
        open={modalOpen}
        onClose={() => setModalOpen(false)}
        onSaved={onSaved}
        projects={projects}
        entry={editing}
        defaultDate={selectedKey}
      />
    </>
  );
}

function WeekGrid({
  days,
  entries,
  loading,
  onSelectDay,
  onAdd,
}: {
  days: Date[];
  entries: TimeEntry[];
  loading: boolean;
  onSelectDay: (d: Date) => void;
  onAdd: () => void;
}) {
  const rows = useMemo(() => {
    const m = new Map<
      string,
      { name: string; client: string | null; color: string; perDay: Map<string, number> }
    >();
    for (const e of entries) {
      const row = m.get(e.project_id) ?? {
        name: e.project_name,
        client: e.project_client,
        color: e.project_color,
        perDay: new Map<string, number>(),
      };
      row.perDay.set(e.entry_date, (row.perDay.get(e.entry_date) ?? 0) + e.minutes);
      m.set(e.project_id, row);
    }
    return [...m.entries()].sort((a, b) => a[1].name.localeCompare(b[1].name));
  }, [entries]);

  const dayTotal = (key: string) => sumMinutes(entries.filter((e) => e.entry_date === key));

  return (
    <Card className="overflow-x-auto">
      <table className="w-full min-w-[720px] text-sm">
        <thead>
          <tr className="border-b border-stone-200 text-ink-500">
            <th className="px-5 py-3 text-left font-medium">Project</th>
            {days.map((d) => (
              <th key={d.toISOString()} className="w-20 px-2 py-3 text-center font-medium">
                <button
                  onClick={() => onSelectDay(d)}
                  className={clsx(
                    "rounded px-1 hover:text-brand-600",
                    isToday(d) && "text-brand-600",
                  )}
                >
                  {format(d, "EEE")}
                  <div className="text-xs font-normal">{format(d, "d MMM")}</div>
                </button>
              </th>
            ))}
            <th className="w-20 px-3 py-3 text-right font-medium">Total</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-stone-100">
          {loading ? (
            <tr>
              <td colSpan={9} className="py-12 text-center">
                <Spinner className="mx-auto" />
              </td>
            </tr>
          ) : rows.length === 0 ? (
            <tr>
              <td colSpan={9}>
                <EmptyState title="No time tracked this week">
                  <Button className="mt-3" variant="success" onClick={onAdd}>
                    <Plus className="h-4 w-4" /> Track time
                  </Button>
                </EmptyState>
              </td>
            </tr>
          ) : (
            rows.map(([id, row]) => (
              <tr key={id}>
                <td className="px-5 py-3">
                  <div className="flex items-center gap-2">
                    <ColorDot color={row.color} />
                    <span className="font-medium">{row.name}</span>
                    {row.client && <span className="text-ink-500">({row.client})</span>}
                  </div>
                </td>
                {days.map((d) => {
                  const mins = row.perDay.get(toISODate(d)) ?? 0;
                  return (
                    <td key={d.toISOString()} className="px-2 py-3 text-center tabular-nums">
                      {mins ? (
                        <button
                          onClick={() => onSelectDay(d)}
                          className="rounded px-1.5 py-0.5 hover:bg-brand-50"
                        >
                          {formatClock(mins)}
                        </button>
                      ) : (
                        <span className="text-ink-300">—</span>
                      )}
                    </td>
                  );
                })}
                <td className="px-3 py-3 text-right font-semibold tabular-nums">
                  {formatClock([...row.perDay.values()].reduce((a, b) => a + b, 0))}
                </td>
              </tr>
            ))
          )}
        </tbody>
        {!loading && rows.length > 0 && (
          <tfoot>
            <tr className="border-t border-stone-200 bg-stone-50 font-semibold">
              <td className="px-5 py-3">Total</td>
              {days.map((d) => (
                <td key={d.toISOString()} className="px-2 py-3 text-center tabular-nums">
                  {formatClock(dayTotal(toISODate(d)))}
                </td>
              ))}
              <td className="px-3 py-3 text-right text-brand-600 tabular-nums">
                {formatClock(sumMinutes(entries))}
              </td>
            </tr>
          </tfoot>
        )}
      </table>
    </Card>
  );
}
