"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { Logo } from "@/components/Logo";
import { Alert, Button, Field, Input } from "@/components/ui";
import { errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";

export default function LoginPage() {
  const { user, loading, login } = useAuth();
  const router = useRouter();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!loading && user) router.replace(user.role === "admin" ? "/admin" : "/time");
  }, [user, loading, router]);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      const u = await login(email, password);
      router.replace(u.role === "admin" ? "/admin" : "/time");
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  }

  return (
    <main className="flex min-h-screen">
      <div className="hidden flex-1 flex-col justify-between bg-ink-900 p-12 text-white lg:flex">
        <Logo light />
        <div>
          <h1 className="max-w-md text-4xl leading-tight font-semibold">
            Know where your team&apos;s time goes.
          </h1>
          <p className="mt-4 max-w-md text-stone-300">
            Log hours against projects in seconds, review your day, week and month at a glance, and
            give admins a clear picture of who worked on what.
          </p>
        </div>
        <p className="text-sm text-stone-400">© {new Date().getFullYear()} Hourwise</p>
      </div>

      <div className="flex flex-1 items-center justify-center p-6">
        <div className="w-full max-w-sm">
          <div className="mb-8 lg:hidden">
            <Logo />
          </div>
          <h2 className="text-2xl font-semibold tracking-tight">Sign in</h2>
          <p className="mt-1 text-sm text-ink-500">
            Use the email and password your admin set up for you.
          </p>
          <form onSubmit={submit} className="mt-6 space-y-4">
            {error && <Alert>{error}</Alert>}
            <Field label="Email">
              <Input
                type="email"
                autoComplete="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                required
                autoFocus
              />
            </Field>
            <Field label="Password">
              <Input
                type="password"
                autoComplete="current-password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required
              />
            </Field>
            <Button type="submit" size="lg" className="w-full" loading={busy}>
              Sign in
            </Button>
          </form>
        </div>
      </div>
    </main>
  );
}
