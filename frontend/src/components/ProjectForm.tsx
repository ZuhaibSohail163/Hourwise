"use client";

import { useState } from "react";
import { api, errorMessage } from "@/lib/api";
import type { Project } from "@/lib/types";
import { Alert, Button, Field, Input, Modal, Textarea } from "./ui";

export const PROJECT_COLORS = [
  "#f36c00",
  "#e11d48",
  "#db2777",
  "#9333ea",
  "#4f46e5",
  "#2563eb",
  "#0891b2",
  "#059669",
  "#65a30d",
  "#ca8a04",
];

interface ProjectFormProps {
  open: boolean;
  onClose: () => void;
  onSaved: (p: Project) => void;
  project?: Project | null;
}

/** Create (no `project`) or edit (with `project`) a project. */
export function ProjectForm(props: ProjectFormProps) {
  const { open, onClose, project } = props;
  return (
    <Modal open={open} onClose={onClose} title={project ? "Edit project" : "New project"}>
      {/* Mounted only while open, so the form state starts fresh every time. */}
      <ProjectFormBody key={project?.id ?? "new"} {...props} />
    </Modal>
  );
}

function ProjectFormBody({ onClose, onSaved, project }: ProjectFormProps) {
  const [name, setName] = useState(project?.name ?? "");
  const [client, setClient] = useState(project?.client ?? "");
  const [description, setDescription] = useState(project?.description ?? "");
  const [color, setColor] = useState(
    () => project?.color ?? PROJECT_COLORS[Math.floor(Math.random() * PROJECT_COLORS.length)],
  );
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    setError(null);
    try {
      const body = { name, client, description, color };
      const saved = project
        ? await api<Project>(`/admin/projects/${project.id}`, { method: "PATCH", body })
        : await api<Project>("/admin/projects", { method: "POST", body });
      onSaved(saved);
      onClose();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setSaving(false);
    }
  }

  return (
    <form onSubmit={submit} className="space-y-4">
      {error && <Alert>{error}</Alert>}
      <Field label="Project name">
        <Input
          value={name}
          onChange={(e) => setName(e.target.value)}
          required
          maxLength={120}
          autoFocus
        />
      </Field>
      <Field label="Client" hint="Optional">
        <Input value={client} onChange={(e) => setClient(e.target.value)} maxLength={120} />
      </Field>
      <Field label="Description" hint="Optional">
        <Textarea rows={3} value={description} onChange={(e) => setDescription(e.target.value)} />
      </Field>
      <div>
        <span className="mb-1.5 block text-sm font-medium text-ink-700">Color</span>
        <div className="flex flex-wrap gap-2">
          {PROJECT_COLORS.map((c) => (
            <button
              type="button"
              key={c}
              onClick={() => setColor(c)}
              className={`h-7 w-7 rounded-full ring-offset-2 transition ${color === c ? "ring-2 ring-ink-900" : ""}`}
              style={{ backgroundColor: c }}
              aria-label={`Color ${c}`}
            />
          ))}
        </div>
      </div>
      <div className="flex justify-end gap-2 pt-1">
        <Button type="button" variant="secondary" onClick={onClose}>
          Cancel
        </Button>
        <Button type="submit" loading={saving}>
          {project ? "Save project" : "Create project"}
        </Button>
      </div>
    </form>
  );
}
