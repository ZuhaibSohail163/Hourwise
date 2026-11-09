"use client";

import { Plus, Search, Users } from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import {
  Alert,
  Avatar,
  Badge,
  Button,
  Card,
  EmptyState,
  Field,
  Input,
  Modal,
  PageHeader,
  PageLoader,
  Select,
} from "@/components/ui";
import { api, errorMessage } from "@/lib/api";
import { useApiData } from "@/lib/hooks";
import type { ProjectSummary, Role, UserWithStats } from "@/lib/types";
import { formatClock } from "@/lib/time";

export default function AdminUsersPage() {
  const { data: users, error, reload } = useApiData<UserWithStats[]>("/admin/users");
  const [query, setQuery] = useState("");
  const [creating, setCreating] = useState(false);

  const q = query.trim().toLowerCase();
  const visible = (users ?? []).filter(
    (u) => !q || u.name.toLowerCase().includes(q) || u.email.toLowerCase().includes(q),
  );

  return (
    <>
      <PageHeader
        title="Team"
        subtitle="Everyone who can sign in to Hourwise."
        action={
          <Button onClick={() => setCreating(true)}>
            <Plus className="h-4 w-4" /> Add person
          </Button>
        }
      />
      {error && <Alert>{error}</Alert>}
      <div className="mb-4">
        <div className="relative w-fit">
          <Search className="pointer-events-none absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-ink-300" />
          <Input
            placeholder="Search by name or email"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="w-72 pl-9"
          />
        </div>
      </div>

      {!users ? (
        <PageLoader />
      ) : (
        <Card className="overflow-x-auto">
          {visible.length === 0 ? (
            <EmptyState icon={<Users className="h-9 w-9" />} title="No people found" />
          ) : (
            <table className="w-full min-w-[720px] text-sm">
              <thead>
                <tr className="border-b border-stone-200 text-left text-ink-500">
                  <th className="px-5 py-3 font-medium">Name</th>
                  <th className="px-5 py-3 font-medium">Role</th>
                  <th className="px-5 py-3 font-medium">Status</th>
                  <th className="px-5 py-3 text-right font-medium">Projects</th>
                  <th className="px-5 py-3 text-right font-medium">Total hours</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-stone-100">
                {visible.map((u) => (
                  <tr key={u.id} className="hover:bg-stone-50">
                    <td className="px-5 py-3">
                      <Link href={`/admin/users/${u.id}`} className="flex items-center gap-3">
                        <Avatar name={u.name} url={u.avatar_url} size={34} />
                        <div className="min-w-0">
                          <div className="font-medium hover:text-brand-600">{u.name}</div>
                          <div className="truncate text-xs text-ink-500">
                            {u.email}
                            {u.job_title ? ` · ${u.job_title}` : ""}
                          </div>
                        </div>
                      </Link>
                    </td>
                    <td className="px-5 py-3">
                      <Badge tone={u.role === "admin" ? "brand" : "neutral"}>
                        {u.role === "admin" ? "Admin" : "Member"}
                      </Badge>
                    </td>
                    <td className="px-5 py-3">
                      <Badge tone={u.is_active ? "green" : "red"}>
                        {u.is_active ? "Active" : "Inactive"}
                      </Badge>
                    </td>
                    <td className="px-5 py-3 text-right tabular-nums">{u.project_count}</td>
                    <td className="px-5 py-3 text-right font-semibold tabular-nums">
                      {formatClock(u.total_minutes)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Card>
      )}

      <CreateUserModal open={creating} onClose={() => setCreating(false)} onCreated={reload} />
    </>
  );
}

interface CreateUserProps {
  open: boolean;
  onClose: () => void;
  onCreated: () => void;
}

function CreateUserModal(props: CreateUserProps) {
  return (
    <Modal open={props.open} onClose={props.onClose} title="Add a person" wide>
      {/* Mounted only while open, so the form starts empty every time. */}
      <CreateUserForm {...props} />
    </Modal>
  );
}

function CreateUserForm({ onClose, onCreated }: CreateUserProps) {
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [role, setRole] = useState<Role>("member");
  const [jobTitle, setJobTitle] = useState("");
  const allProjects = useApiData<ProjectSummary[]>("/admin/projects").data;
  const projects = (allProjects ?? []).filter((p) => !p.is_archived);
  const [projectIds, setProjectIds] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    setError(null);
    try {
      await api("/admin/users", {
        method: "POST",
        body: { name, email, password, role, job_title: jobTitle, project_ids: [...projectIds] },
      });
      onCreated();
      onClose();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setSaving(false);
    }
  }

  return (
    <>
      <form onSubmit={submit} className="space-y-4">
        {error && <Alert>{error}</Alert>}
        <div className="grid gap-4 sm:grid-cols-2">
          <Field label="Full name">
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              required
              maxLength={100}
              autoFocus
            />
          </Field>
          <Field label="Email">
            <Input type="email" value={email} onChange={(e) => setEmail(e.target.value)} required />
          </Field>
          <Field
            label="Initial password"
            hint="At least 8 characters. Share it with them securely — they can change it from their profile."
          >
            <Input
              type="text"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              minLength={8}
              required
              autoComplete="off"
            />
          </Field>
          <Field label="Job title" hint="Optional">
            <Input value={jobTitle} onChange={(e) => setJobTitle(e.target.value)} maxLength={100} />
          </Field>
          <Field label="Role">
            <Select value={role} onChange={(e) => setRole(e.target.value as Role)}>
              <option value="member">Member — tracks their own time</option>
              <option value="admin">Admin — manages projects, people & reports</option>
            </Select>
          </Field>
        </div>
        {projects.length > 0 && (
          <div>
            <span className="mb-1.5 block text-sm font-medium text-ink-700">Add to projects</span>
            <div className="flex flex-wrap gap-2">
              {projects.map((p) => {
                const on = projectIds.has(p.id);
                return (
                  <button
                    type="button"
                    key={p.id}
                    onClick={() =>
                      setProjectIds((prev) => {
                        const next = new Set(prev);
                        if (on) next.delete(p.id);
                        else next.add(p.id);
                        return next;
                      })
                    }
                    className={`inline-flex items-center gap-2 rounded-full border px-3 py-1 text-sm transition ${
                      on
                        ? "border-brand-500 bg-brand-50 text-brand-800"
                        : "border-stone-300 hover:bg-stone-50"
                    }`}
                  >
                    <span className="h-2 w-2 rounded-full" style={{ backgroundColor: p.color }} />
                    {p.name}
                  </button>
                );
              })}
            </div>
          </div>
        )}
        <div className="flex justify-end gap-2 pt-1">
          <Button type="button" variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" loading={saving}>
            Add person
          </Button>
        </div>
      </form>
    </>
  );
}
