export type Role = "admin" | "member";

export interface User {
  id: string;
  email: string;
  name: string;
  role: Role;
  job_title: string | null;
  avatar_url: string | null;
  is_active: boolean;
  created_at: string;
  updated_at: string;
}

export interface UserWithStats extends User {
  project_count: number;
  total_minutes: number;
}

export interface Project {
  id: string;
  name: string;
  client: string | null;
  description: string | null;
  color: string;
  is_archived: boolean;
  created_at: string;
  updated_at: string;
}

export interface ProjectSummary extends Project {
  member_count: number;
  total_minutes: number;
}

export interface UserProject extends Project {
  added_at: string;
  total_minutes: number;
}

export interface ProjectMember {
  id: string;
  name: string;
  email: string;
  avatar_url: string | null;
  role: Role;
  job_title: string | null;
  is_active: boolean;
  added_at: string;
  total_minutes: number;
}

export interface TimeEntry {
  id: string;
  user_id: string;
  project_id: string;
  entry_date: string; // YYYY-MM-DD
  minutes: number;
  description: string;
  created_at: string;
  updated_at: string;
  project_name: string;
  project_color: string;
  project_client: string | null;
  user_name: string;
  user_email: string;
}

export interface AdminStats {
  users: number;
  active_users: number;
  projects: number;
  active_projects: number;
  minutes_today: number;
  minutes_this_week: number;
  minutes_this_month: number;
}
