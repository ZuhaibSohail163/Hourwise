"use client";

import { format } from "date-fns";
import {
  Archive,
  ArchiveRestore,
  ArrowLeft,
  Pencil,
  Plus,
  Search,
  Trash2,
  UserMinus,
  Users,
} from "lucide-react";
import Link from "next/link";
import { useParams, useRouter } from "next/navigation";
import { useState } from "react";
import { PeriodNav } from "@/components/PeriodNav";
import { ProjectForm } from "@/components/ProjectForm";
import { SummaryReport } from "@/components/SummaryReport";
import {
  Alert,
  Avatar,
  Badge,
  Button,
  Card,
  CardHeader,
  ColorDot,
  EmptyState,
  Input,
  Modal,
  PageLoader,
  Segmented,
  Select,
  StatCard,
} from "@/components/ui";
import { api, errorMessage } from "@/lib/api";
import { useApiData, usePeriod } from "@/lib/hooks";
import type { ProjectMember, ProjectSummary, TimeEntry, UserWithStats } from "@/lib/types";
import { formatClock, formatDecimal } from "@/lib/time";

interface Detail {
  project: ProjectSummary;
  members: ProjectMember[];
}

export default function AdminProjectDetail() {
  const { id } = useParams<{ id: string }>();
  const router = useRouter();
  const {
    data: detail,
    error: loadError,
    reload: load,
  } = useApiData<Detail>(`/admin/projects/${id}`);
  const [actionError, setError] = useState<string | null>(null);
  const error = actionError ?? loadError;
  const [tab, setTab] = useState<"members" | "time">("members");
  const [editing, setEditing] = useState(false);
  const [adding, setAdding] = useState(false);

  if (loadError && !detail) return <Alert>{loadError}</Alert>;
  if (!detail) return <PageLoader />;
  const { project, members } = detail;

  async function toggleArchive() {
    try {
      await api(`/admin/projects/${id}`, {
        method: "PATCH",
        body: { is_archived: !project.is_archived },
      });
      load();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  async function remove() {
    if (
      !confirm(
        `Delete "${project.name}" permanently? All ${formatClock(project.total_minutes)} hours logged to it will be deleted too. Consider archiving instead.`,
      )
    )
      return;
    try {
      await api(`/admin/projects/${id}`, { method: "DELETE" });
      router.replace("/admin/projects");
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  async function removeMember(m: ProjectMember) {
    if (!confirm(`Remove ${m.name} from ${project.name}? Their logged time is kept.`)) return;
    try {
      await api(`/admin/projects/${id}/members/${m.id}`, { method: "DELETE" });
      load();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <>
      <Link
        href="/admin/projects"
        className="mb-4 inline-flex items-center gap-1 text-sm text-ink-500 hover:text-ink-900"
      >
        <ArrowLeft className="h-4 w-4" /> All projects
      </Link>

      <div className="mb-6 flex flex-wrap items-start justify-between gap-4">
        <div>
          <div className="flex items-center gap-3">
            <ColorDot color={project.color} className="h-4 w-4" />
            <h1 className="text-2xl font-semibold tracking-tight">{project.name}</h1>
            {project.is_archived && <Badge>Archived</Badge>}
          </div>
          {project.client && <p className="mt-1 text-sm text-ink-500">Client: {project.client}</p>}
          {project.description && (
            <p className="mt-2 max-w-2xl text-sm text-ink-700">{project.description}</p>
          )}
        </div>
        <div className="flex flex-wrap gap-2">
          <Button variant="secondary" onClick={() => setEditing(true)}>
            <Pencil className="h-4 w-4" /> Edit
          </Button>
          <Button variant="secondary" onClick={toggleArchive}>
            {project.is_archived ? (
              <ArchiveRestore className="h-4 w-4" />
            ) : (
              <Archive className="h-4 w-4" />
            )}
            {project.is_archived ? "Unarchive" : "Archive"}
          </Button>
          <Button variant="ghost" onClick={remove} className="text-red-600 hover:bg-red-50">
            <Trash2 className="h-4 w-4" /> Delete
          </Button>
        </div>
      </div>

      {error && (
        <div className="mb-4">
          <Alert>{error}</Alert>
        </div>
      )}

      <div className="mb-6 grid grid-cols-2 gap-4 lg:grid-cols-3">
        <StatCard label="Members" value={project.member_count} />
        <StatCard
          label="Total time"
          value={formatClock(project.total_minutes)}
          hint={`${formatDecimal(project.total_minutes)} hours`}
          accent
        />
        <StatCard label="Created" value={format(new Date(project.created_at), "d MMM yyyy")} />
      </div>

      <div className="mb-4">
        <Segmented
          value={tab}
          onChange={setTab}
          options={[
            { value: "members", label: `Members (${members.length})` },
            { value: "time", label: "Time & work" },
          ]}
        />
      </div>

      {tab === "members" ? (
        <Card>
          <CardHeader
            title="Team members"
            subtitle="Only members can track time to this project."
            action={
              <Button size="sm" onClick={() => setAdding(true)}>
                <Plus className="h-4 w-4" /> Add members
              </Button>
            }
          />
          {members.length === 0 ? (
            <EmptyState icon={<Users className="h-8 w-8" />} title="No members yet">
              Add people so they can start logging time.
            </EmptyState>
          ) : (
            <ul className="divide-y divide-stone-100">
              {members.map((m) => {
                const pct = project.total_minutes
                  ? Math.round((m.total_minutes / project.total_minutes) * 100)
                  : 0;
                return (
                  <li key={m.id} className="flex items-center gap-4 px-5 py-3">
                    <Avatar name={m.name} url={m.avatar_url} size={36} />
                    <div className="min-w-0 flex-1">
                      <Link
                        href={`/admin/users/${m.id}`}
                        className="font-medium hover:text-brand-600"
                      >
                        {m.name}
                      </Link>
                      {!m.is_active && (
                        <span className="ml-2">
                          <Badge tone="red">Inactive</Badge>
                        </span>
                      )}
                      <div className="truncate text-xs text-ink-500">
                        {m.email}
                        {m.job_title ? ` · ${m.job_title}` : ""} · added{" "}
                        {format(new Date(m.added_at), "d MMM yyyy")}
                      </div>
                    </div>
                    <div className="text-right">
                      <div className="font-semibold tabular-nums">
                        {formatClock(m.total_minutes)}
                      </div>
                      <div className="text-xs text-ink-500">{pct}% of project</div>
                    </div>
                    <button
                      onClick={() => removeMember(m)}
                      className="rounded-md p-2 text-ink-500 hover:bg-red-50 hover:text-red-600"
                      title="Remove from project"
                      aria-label={`Remove ${m.name}`}
                    >
                      <UserMinus className="h-4 w-4" />
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </Card>
      ) : (
        <ProjectTime projectId={id} members={members} projectName={project.name} />
      )}

      <ProjectForm
        open={editing}
        onClose={() => setEditing(false)}
        onSaved={() => load()}
        project={project}
      />
      <AddMembersModal
        open={adding}
        onClose={() => setAdding(false)}
        projectId={id}
        existing={members.map((m) => m.id)}
        onAdded={load}
      />
    </>
  );
}

function ProjectTime({
  projectId,
  members,
  projectName,
}: {
  projectId: string;
  members: ProjectMember[];
  projectName: string;
}) {
  const { period, setPeriod, anchor, setAnchor, from, to, fromKey, toKey } = usePeriod("month");
  const [userId, setUserId] = useState("");
  const { data: entries, error } = useApiData<TimeEntry[]>("/admin/time-entries", {
    from: fromKey,
    to: toKey,
    project_id: projectId,
    user_id: userId || undefined,
  });

  return (
    <>
      <div className="mb-6 flex flex-wrap items-center justify-between gap-3">
        <PeriodNav
          period={period}
          anchor={anchor}
          onPeriodChange={setPeriod}
          onAnchorChange={setAnchor}
        />
        <Select value={userId} onChange={(e) => setUserId(e.target.value)} className="w-56">
          <option value="">All members</option>
          {members.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
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
          showMembers
          showProjects={false}
          csvName={`${projectName.replace(/[^\w-]+/g, "_")}-${fromKey}-to-${toKey}.csv`}
        />
      )}
    </>
  );
}

interface AddMembersProps {
  open: boolean;
  onClose: () => void;
  projectId: string;
  existing: string[];
  onAdded: () => void;
}

function AddMembersModal(props: AddMembersProps) {
  return (
    <Modal open={props.open} onClose={props.onClose} title="Add members">
      {/* Mounted only while open, so selection and search start fresh every time. */}
      <AddMembersBody {...props} />
    </Modal>
  );
}

function AddMembersBody({ onClose, projectId, existing, onAdded }: AddMembersProps) {
  const usersQuery = useApiData<UserWithStats[]>("/admin/users");
  const users = usersQuery.data ?? [];
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [saving, setSaving] = useState(false);
  const [saveError, setError] = useState<string | null>(null);
  const error = saveError ?? usersQuery.error;

  const q = query.trim().toLowerCase();
  const candidates = users.filter(
    (u) =>
      u.is_active &&
      !existing.includes(u.id) &&
      (!q || u.name.toLowerCase().includes(q) || u.email.toLowerCase().includes(q)),
  );

  function toggle(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function save() {
    setSaving(true);
    try {
      await api(`/admin/projects/${projectId}/members`, {
        method: "POST",
        body: { user_ids: [...selected] },
      });
      onAdded();
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <>
      {error && (
        <div className="mb-3">
          <Alert>{error}</Alert>
        </div>
      )}
      <div className="relative mb-3">
        <Search className="pointer-events-none absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-ink-300" />
        <Input
          placeholder="Search people"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          className="pl-9"
          autoFocus
        />
      </div>
      {candidates.length === 0 ? (
        <p className="py-6 text-center text-sm text-ink-500">
          Everyone active is already on this project.{" "}
          <Link href="/admin/users" className="text-brand-600 hover:underline">
            Invite someone new
          </Link>
        </p>
      ) : (
        <ul className="max-h-80 divide-y divide-stone-100 overflow-y-auto rounded-lg border border-stone-200">
          {candidates.map((u) => (
            <li key={u.id}>
              <label className="flex cursor-pointer items-center gap-3 px-3 py-2.5 hover:bg-stone-50">
                <input
                  type="checkbox"
                  checked={selected.has(u.id)}
                  onChange={() => toggle(u.id)}
                  className="h-4 w-4 accent-brand-500"
                />
                <Avatar name={u.name} url={u.avatar_url} size={28} />
                <div className="min-w-0">
                  <div className="truncate text-sm font-medium">{u.name}</div>
                  <div className="truncate text-xs text-ink-500">{u.email}</div>
                </div>
              </label>
            </li>
          ))}
        </ul>
      )}
      <div className="mt-4 flex justify-end gap-2">
        <Button variant="secondary" onClick={onClose}>
          Cancel
        </Button>
        <Button onClick={save} disabled={selected.size === 0} loading={saving}>
          Add {selected.size || ""} member{selected.size === 1 ? "" : "s"}
        </Button>
      </div>
    </>
  );
}
