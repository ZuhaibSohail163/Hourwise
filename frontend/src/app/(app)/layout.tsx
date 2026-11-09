"use client";

import clsx from "clsx";
import { LogOut, Shield } from "lucide-react";
import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect } from "react";
import { Logo } from "@/components/Logo";
import { Avatar, PageLoader } from "@/components/ui";
import { useAuth } from "@/lib/auth";

const NAV = [
  { href: "/time", label: "Time" },
  { href: "/reports", label: "Reports" },
  { href: "/profile", label: "Profile" },
];

/** Member-facing shell: horizontal top navigation. */
export default function AppLayout({ children }: { children: React.ReactNode }) {
  const { user, loading, logout } = useAuth();
  const pathname = usePathname();
  const router = useRouter();

  useEffect(() => {
    if (!loading && !user) router.replace("/login");
  }, [user, loading, router]);

  if (loading || !user) return <PageLoader />;

  return (
    <div className="min-h-screen">
      <header className="sticky top-0 z-30 border-b border-stone-200 bg-white">
        <div className="mx-auto flex h-14 max-w-6xl items-center gap-6 px-4">
          <Link href="/time">
            <Logo />
          </Link>
          <nav className="flex items-center gap-1">
            {NAV.map((n) => (
              <Link
                key={n.href}
                href={n.href}
                className={clsx(
                  "rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
                  pathname.startsWith(n.href)
                    ? "bg-brand-50 text-brand-700"
                    : "text-ink-700 hover:bg-stone-100",
                )}
              >
                {n.label}
              </Link>
            ))}
          </nav>
          <div className="ml-auto flex items-center gap-2">
            {user.role === "admin" && (
              <Link
                href="/admin"
                className="hidden items-center gap-1.5 rounded-md bg-ink-900 px-3 py-1.5 text-sm font-medium text-white hover:bg-ink-700 sm:inline-flex"
              >
                <Shield className="h-4 w-4" /> Admin
              </Link>
            )}
            <Link
              href="/profile"
              className="flex items-center gap-2 rounded-md px-2 py-1 hover:bg-stone-100"
            >
              <Avatar name={user.name} url={user.avatar_url} size={30} />
              <span className="hidden text-sm font-medium md:inline">{user.name}</span>
            </Link>
            <button
              onClick={logout}
              className="rounded-md p-2 text-ink-500 hover:bg-stone-100 hover:text-ink-900"
              title="Sign out"
              aria-label="Sign out"
            >
              <LogOut className="h-4 w-4" />
            </button>
          </div>
        </div>
      </header>
      <main className="mx-auto max-w-6xl px-4 py-8">{children}</main>
    </div>
  );
}
