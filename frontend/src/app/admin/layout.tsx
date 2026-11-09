"use client";

import clsx from "clsx";
import { ArrowLeft, ChartColumn, FolderKanban, LayoutDashboard, LogOut, Users } from "lucide-react";
import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect } from "react";
import { Logo } from "@/components/Logo";
import { Avatar, PageLoader } from "@/components/ui";
import { useAuth } from "@/lib/auth";

const NAV = [
  { href: "/admin", label: "Overview", icon: LayoutDashboard, exact: true },
  { href: "/admin/projects", label: "Projects", icon: FolderKanban },
  { href: "/admin/users", label: "Team", icon: Users },
  { href: "/admin/reports", label: "Reports", icon: ChartColumn },
];

/** Admin shell: dark sidebar, visually distinct from the member interface. */
export default function AdminLayout({ children }: { children: React.ReactNode }) {
  const { user, loading, logout } = useAuth();
  const pathname = usePathname();
  const router = useRouter();

  useEffect(() => {
    if (loading) return;
    if (!user) router.replace("/login");
    else if (user.role !== "admin") router.replace("/time");
  }, [user, loading, router]);

  if (loading || !user || user.role !== "admin") return <PageLoader />;

  return (
    <div className="min-h-screen lg:flex">
      <aside className="flex flex-col bg-ink-900 text-stone-200 lg:sticky lg:top-0 lg:h-screen lg:w-60 lg:shrink-0">
        <div className="flex items-center justify-between px-5 py-4">
          <Link href="/admin">
            <Logo light />
          </Link>
          <span className="rounded bg-brand-500 px-1.5 py-0.5 text-[10px] font-bold tracking-wider text-white uppercase">
            Admin
          </span>
        </div>
        <nav className="flex gap-1 overflow-x-auto px-3 pb-3 lg:flex-1 lg:flex-col lg:pb-0">
          {NAV.map((n) => {
            const active = n.exact ? pathname === n.href : pathname.startsWith(n.href);
            return (
              <Link
                key={n.href}
                href={n.href}
                className={clsx(
                  "flex shrink-0 items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors",
                  active
                    ? "bg-white/10 text-white"
                    : "text-stone-300 hover:bg-white/5 hover:text-white",
                )}
              >
                <n.icon className="h-4 w-4" />
                {n.label}
              </Link>
            );
          })}
          <Link
            href="/time"
            className="flex shrink-0 items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium text-stone-300 hover:bg-white/5 hover:text-white lg:mt-4"
          >
            <ArrowLeft className="h-4 w-4" />
            My timesheet
          </Link>
        </nav>
        <div className="hidden items-center gap-3 border-t border-white/10 px-4 py-3 lg:flex">
          <Avatar name={user.name} url={user.avatar_url} size={32} />
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-medium text-white">{user.name}</div>
            <div className="truncate text-xs text-stone-400">{user.email}</div>
          </div>
          <button
            onClick={logout}
            className="rounded-md p-1.5 text-stone-400 hover:bg-white/10 hover:text-white"
            title="Sign out"
            aria-label="Sign out"
          >
            <LogOut className="h-4 w-4" />
          </button>
        </div>
      </aside>
      <main className="min-w-0 flex-1 px-4 py-8 lg:px-10">
        <div className="mx-auto max-w-6xl">{children}</div>
      </main>
    </div>
  );
}
