"use client";

import { useState } from "react";
import { api, errorMessage } from "@/lib/api";
import type { Project, TimeEntry } from "@/lib/types";
import { Alert, Button, Field, Input, Modal, Select, Textarea } from "./ui";

interface EntryModalProps {
  open: boolean;
  onClose: () => void;
  onSaved: (entry: TimeEntry) => void;
  projects: Project[];
  entry?: TimeEntry | null;
  defaultDate: string;
}

/** Create or edit a time entry: project, date, hours + minutes, notes. */
export function EntryModal(props: EntryModalProps) {
  const { open, onClose, entry } = props;
  return (
    <Modal open={open} onClose={onClose} title={entry ? "Edit time entry" : "New time entry"}>
      {/* Mounted only while open, so the form state starts fresh every time. */}
      <EntryForm key={entry?.id ?? "new"} {...props} />
    </Modal>
  );
}

function EntryForm({ onClose, onSaved, projects, entry, defaultDate }: EntryModalProps) {
  const selectable = projects.filter((p) => !p.is_archived || p.id === entry?.project_id);

  const [projectId, setProjectId] = useState(entry?.project_id ?? selectable[0]?.id ?? "");
  const [date, setDate] = useState(entry?.entry_date ?? defaultDate);
  const [hours, setHours] = useState(entry ? String(Math.floor(entry.minutes / 60)) : "");
  const [minutes, setMinutes] = useState(entry ? String(entry.minutes % 60) : "");
  const [description, setDescription] = useState(entry?.description ?? "");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    const h = Number(hours || 0);
    const m = Number(minutes || 0);
    if (!Number.isInteger(h) || !Number.isInteger(m) || h < 0 || m < 0 || m > 59 || h > 24) {
      setError("Enter whole hours (0–24) and minutes (0–59).");
      return;
    }
    const total = h * 60 + m;
    if (total <= 0) {
      setError("Please enter some time.");
      return;
    }
    if (total > 24 * 60) {
      setError("An entry cannot exceed 24 hours.");
      return;
    }
    if (!projectId) {
      setError("Please choose a project.");
      return;
    }
    setSaving(true);
    try {
      const body = { project_id: projectId, entry_date: date, minutes: total, description };
      const saved = entry
        ? await api<TimeEntry>(`/time-entries/${entry.id}`, { method: "PATCH", body })
        : await api<TimeEntry>("/time-entries", { method: "POST", body });
      onSaved(saved);
      onClose();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setSaving(false);
    }
  }

  if (selectable.length === 0) {
    return (
      <Alert tone="info">
        You haven&apos;t been added to any active projects yet. Ask an administrator to add you to a
        project before tracking time.
      </Alert>
    );
  }

  return (
    <form onSubmit={submit} className="space-y-4">
      {error && <Alert>{error}</Alert>}
      <Field label="Project">
        <Select value={projectId} onChange={(e) => setProjectId(e.target.value)} required>
          {selectable.map((p) => (
            <option key={p.id} value={p.id}>
              {p.client ? `${p.client} — ${p.name}` : p.name}
              {p.is_archived ? " (archived)" : ""}
            </option>
          ))}
        </Select>
      </Field>
      <div className="grid grid-cols-3 gap-3">
        <Field label="Date" className="col-span-3 sm:col-span-1">
          <Input type="date" value={date} onChange={(e) => setDate(e.target.value)} required />
        </Field>
        <Field label="Hours">
          <Input
            type="number"
            inputMode="numeric"
            min={0}
            max={24}
            placeholder="0"
            value={hours}
            onChange={(e) => setHours(e.target.value)}
            autoFocus
          />
        </Field>
        <Field label="Minutes">
          <Input
            type="number"
            inputMode="numeric"
            min={0}
            max={59}
            step={1}
            placeholder="00"
            value={minutes}
            onChange={(e) => setMinutes(e.target.value)}
          />
        </Field>
      </div>
      <Field label="Notes" hint="What did you work on? This appears in your summaries and reports.">
        <Textarea
          rows={4}
          maxLength={5000}
          placeholder="e.g. Implemented the login page and fixed validation bugs"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
        />
      </Field>
      <div className="flex justify-end gap-2 pt-1">
        <Button type="button" variant="secondary" onClick={onClose}>
          Cancel
        </Button>
        <Button type="submit" variant="success" loading={saving}>
          {entry ? "Update entry" : "Save entry"}
        </Button>
      </div>
    </form>
  );
}
