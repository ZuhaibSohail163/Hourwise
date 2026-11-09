"use client";

import { format } from "date-fns";
import { ArrowLeft, FolderKanban, KeyRound, Trash2, UserMinus } from "lucide-react";
import Link from "next/link";
import { useParams, useRouter } from "next/navigation";
import { useState } from "react";
import { PeriodNav } from "@/components/PeriodNav";
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
  Field,
  Input,
  PageLoader,
  Segmented,
  Select,
  StatCard,
} from "@/components/ui";
import { api, errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { useApiData, usePeriod } from "@/lib/hooks";
import type { Role, TimeEntry, User, UserProject, UserWithStats } from "@/lib/types";
import { formatClock, formatDecimal } from "@/lib/time";

interface Detail {
  user: UserWithStats;
  projects: UserProject[];
}

type Notice = { tone: "error" | "success"; text: string } | null;

export default function AdminUserDetail() {
  const { id } = useParams<{ id: string }>();
  const router = useRouter();
  const { user: me, setUser: setMe } = useAuth();
  const {
    data: detail,
    error: loadError,
    reload: load,
    mutate: setDetail,
  } = useApiData<Detail>(`/admin/users/${id}`);
  const [actionError, setError] = useState<string | null>(null);
  const error = actionError ?? loadError;
  const [tab, setTab] = useState<"time" | "projects" | "account">("time");

  if (loadError && !detail) return <Alert>{loadError}</Alert>;
  if (!detail) return <PageLoader />;
  const { user, projects } = detail;
  const isSelf = me?.id === user.id;

  async function removeFromProject(p: UserProject) {
    if (!confirm(`Remove ${user.name} from ${p.name}? Their logged time is kept.`)) return;
    try {
      await api(`/admin/projects/${p.id}/members/${user.id}`, { method: "DELETE" });
      load();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  async function deleteUser() {
    if (
      !confirm(
        `Permanently delete ${user.name}? All ${formatClock(user.total_minutes)} hours they logged will be deleted. Deactivating keeps their history instead.`,
      )
    )
      return;
    try {
      await api(`/admin/users/${user.id}`, { method: "DELETE" });
      router.replace("/admin/users");
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <>
      <Link
        href="/admin/users"
        className="mb-4 inline-flex items-center gap-1 text-sm text-ink-500 hover:text-ink-900"
      >
        <ArrowLeft className="h-4 w-4" /> Team
      </Link>

      <div className="mb-6 flex flex-wrap items-center gap-4">
        <Avatar name={user.name} url={user.avatar_url} size={64} />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="text-2xl font-semibold tracking-tight">{user.name}</h1>
            <Badge tone={user.role === "admin" ? "brand" : "neutral"}>
              {user.role === "admin" ? "Admin" : "Member"}
            </Badge>
            <Badge tone={user.is_active ? "green" : "red"}>
              {user.is_active ? "Active" : "Inactive"}
            </Badge>
          </div>
          <p className="text-sm text-ink-500">
            {user.email}
            {user.job_title ? ` · ${user.job_title}` : ""} · joined{" "}
            {format(new Date(user.created_at), "d MMM yyyy")}
          </p>
        </div>
      </div>

      {error && (
        <div className="mb-4">
          <Alert>{error}</Alert>
        </div>
      )}

      <div className="mb-6 grid grid-cols-2 gap-4 lg:grid-cols-3">
        <StatCard
          label="Total time"
          value={formatClock(user.total_minutes)}
          hint={`${formatDecimal(user.total_minutes)} hours`}
          accent
        />
        <StatCard label="Projects" value={user.project_count} />
        <StatCard label="Last updated" value={format(new Date(user.updated_at), "d MMM yyyy")} />
      </div>

      <div className="mb-4">
        <Segmented
          value={tab}
          onChange={setTab}
          options={[
            { value: "time", label: "Time & work" },
            { value: "projects", label: `Projects (${projects.length})` },
            { value: "account", label: "Account" },
          ]}
        />
      </div>

      {tab === "time" && <UserTime userId={user.id} userName={user.name} />}

      {tab === "projects" && (
        <Card>
          <CardHeader title="Projects" subtitle="Manage membership from each project's page." />
          {projects.length === 0 ? (
            <EmptyState icon={<FolderKanban className="h-8 w-8" />} title="Not on any projects">
              <Link href="/admin/projects" className="text-brand-600 hover:underline">
                Go to projects
              </Link>{" "}
              to add them.
            </EmptyState>
          ) : (
            <ul className="divide-y divide-stone-100">
              {projects.map((p) => (
                <li key={p.id} className="flex items-center gap-3 px-5 py-3">
                  <ColorDot color={p.color} />
                  <div className="min-w-0 flex-1">
                    <Link
                      href={`/admin/projects/${p.id}`}
                      className="font-medium hover:text-brand-600"
                    >
                      {p.name}
                    </Link>
                    {p.is_archived && (
                      <span className="ml-2">
                        <Badge>Archived</Badge>
                      </span>
                    )}
                    <div className="text-xs text-ink-500">
                      {p.client ? `${p.client} · ` : ""}member since{" "}
                      {format(new Date(p.added_at), "d MMM yyyy")}
                    </div>
                  </div>
                  <span className="font-semibold tabular-nums">{formatClock(p.total_minutes)}</span>
                  <button
                    onClick={() => removeFromProject(p)}
                    className="rounded-md p-2 text-ink-500 hover:bg-red-50 hover:text-red-600"
                    title="Remove from project"
                    aria-label={`Remove from ${p.name}`}
                  >
                    <UserMinus className="h-4 w-4" />
                  </button>
                </li>
              ))}
            </ul>
          )}
        </Card>
      )}

      {tab === "account" && (
        <div className="space-y-6">
          <AccountForm
            user={user}
            isSelf={isSelf}
            onSaved={(u) => {
              setDetail((d) => ({ ...d, user: { ...d.user, ...u } }));
              if (isSelf) setMe(u);
            }}
          />
          <PasswordReset userId={user.id} />
          {!isSelf && (
            <Card className="border-red-200">
              <CardHeader
                title="Danger zone"
                subtitle="Deleting a person also deletes all time they have logged."
              />
              <div className="flex justify-end p-5">
                <Button variant="danger" onClick={deleteUser}>
                  <Trash2 className="h-4 w-4" /> Delete person
                </Button>
              </div>
            </Card>
          )}
        </div>
      )}
    </>
  );
}

function UserTime({ userId, userName }: { userId: string; userName: string }) {
  const { period, setPeriod, anchor, setAnchor, from, to, fromKey, toKey } = usePeriod("week");
  const { data: entries, error } = useApiData<TimeEntry[]>("/admin/time-entries", {
    from: fromKey,
    to: toKey,
    user_id: userId,
  });

  return (
    <>
      <div className="mb-6">
        <PeriodNav
          period={period}
          anchor={anchor}
          onPeriodChange={setPeriod}
          onAnchorChange={setAnchor}
        />
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
          csvName={`${userName.replace(/[^\w-]+/g, "_")}-${fromKey}-to-${toKey}.csv`}
        />
      )}
    </>
  );
}

function AccountForm({
  user,
  isSelf,
  onSaved,
}: {
  user: User;
  isSelf: boolean;
  onSaved: (u: User) => void;
}) {
  const [name, setName] = useState(user.name);
  const [email, setEmail] = useState(user.email);
  const [jobTitle, setJobTitle] = useState(user.job_title ?? "");
  const [role, setRole] = useState<Role>(user.role);
  const [active, setActive] = useState(user.is_active);
  const [notice, setNotice] = useState<Notice>(null);
  const [saving, setSaving] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    setNotice(null);
    try {
      const updated = await api<User>(`/admin/users/${user.id}`, {
        method: "PATCH",
        body: { name, email, job_title: jobTitle, role, is_active: active },
      });
      onSaved(updated);
      setNotice({ tone: "success", text: "Account updated." });
    } catch (err) {
      setNotice({ tone: "error", text: errorMessage(err) });
    } finally {
      setSaving(false);
    }
  }

  return (
    <Card>
      <CardHeader
        title="Account details"
        subtitle="Only admins can change a person's email, role and status."
      />
      <form onSubmit={submit} className="space-y-4 p-5">
        {notice && <Alert tone={notice.tone}>{notice.text}</Alert>}
        <div className="grid gap-4 sm:grid-cols-2">
          <Field label="Full name">
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              required
              maxLength={100}
            />
          </Field>
          <Field label="Email" hint="This is the address they sign in with.">
            <Input type="email" value={email} onChange={(e) => setEmail(e.target.value)} required />
          </Field>
          <Field label="Job title">
            <Input value={jobTitle} onChange={(e) => setJobTitle(e.target.value)} maxLength={100} />
          </Field>
          <Field label="Role" hint={isSelf ? "You can't remove your own admin role." : undefined}>
            <Select
              value={role}
              onChange={(e) => setRole(e.target.value as Role)}
              disabled={isSelf}
            >
              <option value="member">Member</option>
              <option value="admin">Admin</option>
            </Select>
          </Field>
        </div>
        <label className="flex items-center gap-3 text-sm">
          <input
            type="checkbox"
            checked={active}
            onChange={(e) => setActive(e.target.checked)}
            disabled={isSelf}
            className="h-4 w-4 accent-brand-500"
          />
          <span>
            <span className="font-medium">Active</span>
            <span className="text-ink-500">
              {" "}
              — inactive people can&apos;t sign in, but their time history is kept.
            </span>
          </span>
        </label>
        <div className="flex justify-end">
          <Button type="submit" loading={saving}>
            Save changes
          </Button>
        </div>
      </form>
    </Card>
  );
}

function PasswordReset({ userId }: { userId: string }) {
  const [password, setPassword] = useState("");
  const [notice, setNotice] = useState<Notice>(null);
  const [saving, setSaving] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    setNotice(null);
    try {
      await api(`/admin/users/${userId}`, { method: "PATCH", body: { password } });
      setPassword("");
      setNotice({
        tone: "success",
        text: "Password reset. Share the new password with them securely.",
      });
    } catch (err) {
      setNotice({ tone: "error", text: errorMessage(err) });
    } finally {
      setSaving(false);
    }
  }

  return (
    <Card>
      <CardHeader title="Reset password" subtitle="Set a new temporary password for this person." />
      <form onSubmit={submit} className="flex flex-wrap items-end gap-3 p-5">
        {notice && (
          <div className="w-full">
            <Alert tone={notice.tone}>{notice.text}</Alert>
          </div>
        )}
        <Field label="New password" className="min-w-64 flex-1">
          <Input
            type="text"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            minLength={8}
            required
            autoComplete="off"
          />
        </Field>
        <Button type="submit" variant="secondary" loading={saving}>
          <KeyRound className="h-4 w-4" /> Reset password
        </Button>
      </form>
    </Card>
  );
}
