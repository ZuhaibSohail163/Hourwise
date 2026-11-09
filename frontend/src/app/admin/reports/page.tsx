"use client";

import { useState } from "react";
import { PeriodNav } from "@/components/PeriodNav";
import { SummaryReport } from "@/components/SummaryReport";
import { Alert, PageHeader, PageLoader, Select } from "@/components/ui";
import { useApiData, usePeriod } from "@/lib/hooks";
import type { ProjectSummary, TimeEntry, UserWithStats } from "@/lib/types";

export default function AdminReportsPage() {
  const { period, setPeriod, anchor, setAnchor, from, to, fromKey, toKey } = usePeriod("week");
  const [projectId, setProjectId] = useState("");
  const [userId, setUserId] = useState("");
  const projects = useApiData<ProjectSummary[]>("/admin/projects").data ?? [];
  const users = useApiData<UserWithStats[]>("/admin/users").data ?? [];
  const { data: entries, error } = useApiData<TimeEntry[]>("/admin/time-entries", {
    from: fromKey,
    to: toKey,
    project_id: projectId || undefined,
    user_id: userId || undefined,
  });

  return (
    <>
      <PageHeader title="Reports" subtitle="Time and work across every project and person." />
      <div className="mb-6 flex flex-wrap items-center justify-between gap-3">
        <PeriodNav
          period={period}
          anchor={anchor}
          onPeriodChange={setPeriod}
          onAnchorChange={setAnchor}
        />
        <div className="flex flex-wrap gap-2">
          <Select value={projectId} onChange={(e) => setProjectId(e.target.value)} className="w-48">
            <option value="">All projects</option>
            {projects.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
                {p.is_archived ? " (archived)" : ""}
              </option>
            ))}
          </Select>
          <Select value={userId} onChange={(e) => setUserId(e.target.value)} className="w-48">
            <option value="">Everyone</option>
            {users.map((u) => (
              <option key={u.id} value={u.id}>
                {u.name}
              </option>
            ))}
          </Select>
        </div>
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
          showMembers
          csvName={`hourwise-${fromKey}-to-${toKey}.csv`}
        />
      )}
    </>
  );
}
