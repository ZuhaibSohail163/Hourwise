"use client";

import { FolderKanban, Plus, Search } from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { ProjectForm } from "@/components/ProjectForm";
import {
  Alert,
  Badge,
  Button,
  Card,
  ColorDot,
  EmptyState,
  Input,
  PageHeader,
  PageLoader,
  Segmented,
} from "@/components/ui";
import { useApiData } from "@/lib/hooks";
import type { ProjectSummary } from "@/lib/types";
import { formatClock, formatDecimal } from "@/lib/time";

export default function AdminProjectsPage() {
  const { data: projects, error, reload } = useApiData<ProjectSummary[]>("/admin/projects");
  const [filter, setFilter] = useState<"active" | "archived" | "all">("active");
  const [query, setQuery] = useState("");
  const [creating, setCreating] = useState(false);

  const q = query.trim().toLowerCase();
  const visible = (projects ?? []).filter(
    (p) =>
      (filter === "all" || (filter === "active" ? !p.is_archived : p.is_archived)) &&
      (!q || p.name.toLowerCase().includes(q) || (p.client ?? "").toLowerCase().includes(q)),
  );

  return (
    <>
      <PageHeader
        title="Projects"
        subtitle="Create projects and choose who can track time to them."
        action={
          <Button onClick={() => setCreating(true)}>
            <Plus className="h-4 w-4" /> New project
          </Button>
        }
      />
      {error && <Alert>{error}</Alert>}

      <div className="mb-4 flex flex-wrap items-center gap-3">
        <Segmented
          value={filter}
          onChange={setFilter}
          options={[
            { value: "active", label: "Active" },
            { value: "archived", label: "Archived" },
            { value: "all", label: "All" },
          ]}
        />
        <div className="relative">
          <Search className="pointer-events-none absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-ink-300" />
          <Input
            placeholder="Search projects or clients"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="w-64 pl-9"
          />
        </div>
      </div>

      {!projects ? (
        <PageLoader />
      ) : (
        <Card className="overflow-x-auto">
          {visible.length === 0 ? (
            <EmptyState icon={<FolderKanban className="h-9 w-9" />} title="No projects found">
              {projects.length === 0 && "Create a project, then add team members to it."}
            </EmptyState>
          ) : (
            <table className="w-full min-w-[640px] text-sm">
              <thead>
                <tr className="border-b border-stone-200 text-left text-ink-500">
                  <th className="px-5 py-3 font-medium">Project</th>
                  <th className="px-5 py-3 font-medium">Client</th>
                  <th className="px-5 py-3 text-right font-medium">Members</th>
                  <th className="px-5 py-3 text-right font-medium">Total hours</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-stone-100">
                {visible.map((p) => (
                  <tr key={p.id} className="hover:bg-stone-50">
                    <td className="px-5 py-3">
                      <Link
                        href={`/admin/projects/${p.id}`}
                        className="flex items-center gap-2 font-medium hover:text-brand-600"
                      >
                        <ColorDot color={p.color} />
                        {p.name}
                        {p.is_archived && <Badge>Archived</Badge>}
                      </Link>
                    </td>
                    <td className="px-5 py-3 text-ink-500">{p.client ?? "—"}</td>
                    <td className="px-5 py-3 text-right tabular-nums">{p.member_count}</td>
                    <td className="px-5 py-3 text-right tabular-nums">
                      <span className="font-semibold">{formatClock(p.total_minutes)}</span>
                      <span className="ml-2 text-ink-500">({formatDecimal(p.total_minutes)})</span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Card>
      )}

      <ProjectForm open={creating} onClose={() => setCreating(false)} onSaved={reload} />
    </>
  );
}
