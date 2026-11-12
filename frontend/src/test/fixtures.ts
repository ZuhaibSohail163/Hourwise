import type { Project, ProjectSummary, TimeEntry, User, UserWithStats } from "@/lib/types";
import { toISODate } from "@/lib/time";

const ts = "2026-01-01T00:00:00.000Z";

export function makeUser(over: Partial<User> = {}): User {
  return {
    id: "u1",
    email: "ann@example.com",
    name: "Ann Lee",
    role: "member",
    job_title: null,
    avatar_url: null,
    is_active: true,
    created_at: ts,
    updated_at: ts,
    ...over,
  };
}

export function makeUserWithStats(over: Partial<UserWithStats> = {}): UserWithStats {
  return { ...makeUser(), project_count: 1, total_minutes: 0, ...over };
}

export function makeProject(over: Partial<Project> = {}): Project {
  return {
    id: "p1",
    name: "Website",
    client: "ACME",
    description: null,
    color: "#f36c00",
    is_archived: false,
    created_at: ts,
    updated_at: ts,
    ...over,
  };
}

export function makeProjectSummary(over: Partial<ProjectSummary> = {}): ProjectSummary {
  return { ...makeProject(), member_count: 0, total_minutes: 0, ...over };
}

let n = 0;
export function makeEntry(over: Partial<TimeEntry> = {}): TimeEntry {
  n += 1;
  return {
    id: `e${n}`,
    user_id: "u1",
    project_id: "p1",
    entry_date: toISODate(new Date()),
    minutes: 60,
    description: "Work",
    created_at: `2026-01-01T00:00:${String(n % 60).padStart(2, "0")}.000Z`,
    updated_at: ts,
    project_name: "Website",
    project_color: "#f36c00",
    project_client: "ACME",
    user_name: "Ann Lee",
    user_email: "ann@example.com",
    ...over,
  };
}
