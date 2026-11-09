"use client";

import { Camera, KeyRound, Lock, Trash2 } from "lucide-react";
import { useRef, useState } from "react";
import {
  Alert,
  Avatar,
  Badge,
  Button,
  Card,
  CardHeader,
  Field,
  Input,
  PageHeader,
} from "@/components/ui";
import { api, errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import type { User } from "@/lib/types";

type Notice = { tone: "error" | "success"; text: string } | null;

export default function ProfilePage() {
  const { user, setUser } = useAuth();
  const fileRef = useRef<HTMLInputElement>(null);

  const [name, setName] = useState(user?.name ?? "");
  const [jobTitle, setJobTitle] = useState(user?.job_title ?? "");
  const [profileNotice, setProfileNotice] = useState<Notice>(null);
  const [savingProfile, setSavingProfile] = useState(false);

  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirmPw, setConfirmPw] = useState("");
  const [pwNotice, setPwNotice] = useState<Notice>(null);
  const [savingPw, setSavingPw] = useState(false);

  const [avatarNotice, setAvatarNotice] = useState<Notice>(null);
  const [uploading, setUploading] = useState(false);

  if (!user) return null;

  async function saveProfile(e: React.FormEvent) {
    e.preventDefault();
    setSavingProfile(true);
    setProfileNotice(null);
    try {
      const updated = await api<User>("/me", {
        method: "PATCH",
        body: { name, job_title: jobTitle },
      });
      setUser(updated);
      setProfileNotice({ tone: "success", text: "Profile updated." });
    } catch (err) {
      setProfileNotice({ tone: "error", text: errorMessage(err) });
    } finally {
      setSavingProfile(false);
    }
  }

  async function changePassword(e: React.FormEvent) {
    e.preventDefault();
    setPwNotice(null);
    if (next !== confirmPw) {
      setPwNotice({ tone: "error", text: "New passwords do not match." });
      return;
    }
    setSavingPw(true);
    try {
      await api("/me/password", {
        method: "PUT",
        body: { current_password: current, new_password: next },
      });
      setCurrent("");
      setNext("");
      setConfirmPw("");
      setPwNotice({ tone: "success", text: "Password changed." });
    } catch (err) {
      setPwNotice({ tone: "error", text: errorMessage(err) });
    } finally {
      setSavingPw(false);
    }
  }

  async function uploadAvatar(file: File) {
    setAvatarNotice(null);
    if (file.size > 2 * 1024 * 1024) {
      setAvatarNotice({ tone: "error", text: "Image must be 2 MB or smaller." });
      return;
    }
    setUploading(true);
    try {
      const form = new FormData();
      form.append("file", file);
      setUser(await api<User>("/me/avatar", { method: "POST", form }));
    } catch (err) {
      setAvatarNotice({ tone: "error", text: errorMessage(err) });
    } finally {
      setUploading(false);
      if (fileRef.current) fileRef.current.value = "";
    }
  }

  async function removeAvatar() {
    setAvatarNotice(null);
    try {
      setUser(await api<User>("/me/avatar", { method: "DELETE" }));
    } catch (err) {
      setAvatarNotice({ tone: "error", text: errorMessage(err) });
    }
  }

  return (
    <>
      <PageHeader title="Profile" subtitle="Manage your personal details and password." />
      <div className="grid gap-6 lg:grid-cols-3">
        <Card className="h-fit p-6 text-center">
          <div className="relative mx-auto w-fit">
            <Avatar name={user.name} url={user.avatar_url} size={112} />
            <button
              onClick={() => fileRef.current?.click()}
              className="absolute right-0 bottom-0 rounded-full border-2 border-white bg-brand-500 p-2 text-white shadow hover:bg-brand-600"
              aria-label="Upload profile picture"
            >
              <Camera className="h-4 w-4" />
            </button>
          </div>
          <input
            ref={fileRef}
            type="file"
            accept="image/png,image/jpeg,image/webp,image/gif"
            className="hidden"
            onChange={(e) => e.target.files?.[0] && uploadAvatar(e.target.files[0])}
          />
          <h2 className="mt-4 text-lg font-semibold">{user.name}</h2>
          {user.job_title && <p className="text-sm text-ink-500">{user.job_title}</p>}
          <div className="mt-2">
            <Badge tone={user.role === "admin" ? "brand" : "neutral"}>
              {user.role === "admin" ? "Administrator" : "Member"}
            </Badge>
          </div>
          <div className="mt-5 flex justify-center gap-2">
            <Button
              variant="secondary"
              size="sm"
              onClick={() => fileRef.current?.click()}
              loading={uploading}
            >
              <Camera className="h-4 w-4" /> Change photo
            </Button>
            {user.avatar_url && (
              <Button variant="ghost" size="sm" onClick={removeAvatar}>
                <Trash2 className="h-4 w-4" /> Remove
              </Button>
            )}
          </div>
          <p className="mt-3 text-xs text-ink-500">PNG, JPEG, WEBP or GIF, up to 2 MB.</p>
          {avatarNotice && (
            <div className="mt-3 text-left">
              <Alert tone={avatarNotice.tone}>{avatarNotice.text}</Alert>
            </div>
          )}
        </Card>

        <div className="space-y-6 lg:col-span-2">
          <Card>
            <CardHeader title="Personal details" />
            <form onSubmit={saveProfile} className="space-y-4 p-5">
              {profileNotice && <Alert tone={profileNotice.tone}>{profileNotice.text}</Alert>}
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="Full name">
                  <Input
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    required
                    maxLength={100}
                  />
                </Field>
                <Field label="Job title" hint="Optional">
                  <Input
                    value={jobTitle}
                    onChange={(e) => setJobTitle(e.target.value)}
                    maxLength={100}
                  />
                </Field>
              </div>
              <Field label="Email" hint="Only an administrator can change your email address.">
                <div className="relative">
                  <Input value={user.email} disabled className="pr-9" />
                  <Lock className="absolute top-1/2 right-3 h-4 w-4 -translate-y-1/2 text-ink-300" />
                </div>
              </Field>
              <div className="flex justify-end">
                <Button type="submit" loading={savingProfile}>
                  Save changes
                </Button>
              </div>
            </form>
          </Card>

          <Card>
            <CardHeader title="Change password" subtitle="Use at least 8 characters." />
            <form onSubmit={changePassword} className="space-y-4 p-5">
              {pwNotice && <Alert tone={pwNotice.tone}>{pwNotice.text}</Alert>}
              <Field label="Current password">
                <Input
                  type="password"
                  autoComplete="current-password"
                  value={current}
                  onChange={(e) => setCurrent(e.target.value)}
                  required
                />
              </Field>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="New password">
                  <Input
                    type="password"
                    autoComplete="new-password"
                    value={next}
                    onChange={(e) => setNext(e.target.value)}
                    minLength={8}
                    required
                  />
                </Field>
                <Field label="Confirm new password">
                  <Input
                    type="password"
                    autoComplete="new-password"
                    value={confirmPw}
                    onChange={(e) => setConfirmPw(e.target.value)}
                    minLength={8}
                    required
                  />
                </Field>
              </div>
              <div className="flex justify-end">
                <Button type="submit" variant="secondary" loading={savingPw}>
                  <KeyRound className="h-4 w-4" /> Update password
                </Button>
              </div>
            </form>
          </Card>
        </div>
      </div>
    </>
  );
}
