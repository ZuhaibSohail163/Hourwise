"use client";

import { useState } from "react";
import { PeriodNav } from "@/components/PeriodNav";
import { SummaryReport } from "@/components/SummaryReport";
import { Alert, PageHeader, PageLoader, Select } from "@/components/ui";
import { useApiData, usePeriod } from "@/lib/hooks";
import type { Project, TimeEntry } from "@/lib/types";

export default function MyReportsPage() {
  const { period, setPeriod, anchor, setAnchor, from, to, fromKey, toKey } = usePeriod("week");
  const [projectId, setProjectId] = useState("");
  const projects = useApiData<Project[]>("/projects").data ?? [];
  const { data: entries, error } = useApiData<TimeEntry[]>("/time-entries", {
    from: fromKey,
    to: toKey,
    project_id: projectId || undefined,
  });

  return (
    <>
      <PageHeader title="My reports" subtitle="Your hours and a summary of the work you did." />
      <div className="mb-6 flex flex-wrap items-center justify-between gap-3">
        <PeriodNav
          period={period}
          anchor={anchor}
          onPeriodChange={setPeriod}
          onAnchorChange={setAnchor}
        />
        <Select value={projectId} onChange={(e) => setProjectId(e.target.value)} className="w-56">
          <option value="">All projects</option>
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </Select>
      </div>
      {error && <Alert>{error}</Alert>}
      {!entries ? (
        !error && <PageLoader />
      ) : (
        <SummaryReport
          entries={entries}
          from={from}
          to={to}
          period={period}
          csvName={`my-time-${fromKey}-to-${toKey}.csv`}
        />
      )}
    </>
  );
}
