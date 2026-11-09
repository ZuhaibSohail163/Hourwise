"use client";

import { endOfWeek, startOfWeek } from "date-fns";
import { FolderKanban, Users } from "lucide-react";
import Link from "next/link";
import { useEffect, useState } from "react";
import {
  Alert,
  Avatar,
  Card,
  CardHeader,
  ColorDot,
  EmptyState,
  PageHeader,
  PageLoader,
  StatCard,
} from "@/components/ui";
import { api, errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import type { AdminStats, ProjectSummary, TimeEntry } from "@/lib/types";
import { WEEK_OPTS, formatClock, formatDecimal, toISODate } from "@/lib/time";

export default function AdminOverview() {
  const { user } = useAuth();
  const [stats, setStats] = useState<AdminStats | null>(null);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [weekEntries, setWeekEntries] = useState<TimeEntry[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const now = new Date();
    Promise.all([
      api<AdminStats>("/admin/stats"),
      api<ProjectSummary[]>("/admin/projects"),
      api<TimeEntry[]>("/admin/time-entries", {
        query: {
          from: toISODate(startOfWeek(now, WEEK_OPTS)),
          to: toISODate(endOfWeek(now, WEEK_OPTS)),
        },
      }),
    ])
      .then(([s, p, e]) => {
        setStats(s);
        setProjects(p);
        setWeekEntries(e);
      })
      .catch((e) => setError(errorMessage(e)));
  }, []);

  if (error) return <Alert>{error}</Alert>;
  if (!stats) return <PageLoader />;

  // Hours per member this week.
  const perMember = new Map<string, { name: string; minutes: number }>();
  for (const e of weekEntries) {
    const m = perMember.get(e.user_id) ?? { name: e.user_name, minutes: 0 };
    m.minutes += e.minutes;
    perMember.set(e.user_id, m);
  }
  const members = [...perMember.entries()].sort((a, b) => b[1].minutes - a[1].minutes);
  const activeProjects = projects.filter((p) => !p.is_archived);

  return (
    <>
      <PageHeader
        title={`Welcome back, ${user?.name.split(" ")[0] ?? ""}`}
        subtitle="Here's what your team has been up to."
      />

      <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        <StatCard label="Hours today" value={formatClock(stats.minutes_today)} accent />
        <StatCard
          label="Hours this week"
          value={formatClock(stats.minutes_this_week)}
          hint={`${formatDecimal(stats.minutes_this_week)} h`}
        />
        <StatCard
          label="Hours this month"
          value={formatClock(stats.minutes_this_month)}
          hint={`${formatDecimal(stats.minutes_this_month)} h`}
        />
        <StatCard
          label="Team"
          value={stats.active_users}
          hint={`${stats.active_projects} active project${stats.active_projects === 1 ? "" : "s"}`}
        />
      </div>

      <div className="mt-6 grid gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader
            title="Active projects"
            action={
              <Link
                href="/admin/projects"
                className="text-sm font-medium text-brand-600 hover:underline"
              >
                View all
              </Link>
            }
          />
          {activeProjects.length === 0 ? (
            <EmptyState icon={<FolderKanban className="h-8 w-8" />} title="No projects yet">
              <Link href="/admin/projects" className="text-brand-600 hover:underline">
                Create your first project
              </Link>
            </EmptyState>
          ) : (
            <ul className="divide-y divide-stone-100">
              {activeProjects.slice(0, 8).map((p) => (
                <li key={p.id}>
                  <Link
                    href={`/admin/projects/${p.id}`}
                    className="flex items-center gap-3 px-5 py-3 hover:bg-stone-50"
                  >
                    <ColorDot color={p.color} />
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-sm font-medium">{p.name}</div>
                      <div className="text-xs text-ink-500">
                        {p.client ? `${p.client} · ` : ""}
                        {p.member_count} member{p.member_count === 1 ? "" : "s"}
                      </div>
                    </div>
                    <span className="text-sm font-semibold tabular-nums">
                      {formatClock(p.total_minutes)}
                    </span>
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card>
          <CardHeader
            title="Hours this week by member"
            action={
              <Link
                href="/admin/reports"
                className="text-sm font-medium text-brand-600 hover:underline"
              >
                Reports
              </Link>
            }
          />
          {members.length === 0 ? (
            <EmptyState icon={<Users className="h-8 w-8" />} title="No time tracked this week" />
          ) : (
            <ul className="divide-y divide-stone-100">
              {members.map(([id, m]) => {
                const max = members[0][1].minutes || 1;
                return (
                  <li key={id}>
                    <Link
                      href={`/admin/users/${id}`}
                      className="flex items-center gap-3 px-5 py-3 hover:bg-stone-50"
                    >
                      <Avatar name={m.name} size={28} />
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-sm font-medium">{m.name}</div>
                        <div className="mt-1 h-1.5 rounded-full bg-stone-100">
                          <div
                            className="h-full rounded-full bg-brand-500"
                            style={{ width: `${(m.minutes / max) * 100}%` }}
                          />
                        </div>
                      </div>
                      <span className="text-sm font-semibold tabular-nums">
                        {formatClock(m.minutes)}
                      </span>
                    </Link>
                  </li>
                );
              })}
            </ul>
          )}
        </Card>
      </div>
    </>
  );
}
