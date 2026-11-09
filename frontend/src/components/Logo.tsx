import { Timer } from "lucide-react";

export function Logo({ light = false }: { light?: boolean }) {
  return (
    <span className="inline-flex items-center gap-2">
      <span className="inline-flex h-8 w-8 items-center justify-center rounded-lg bg-brand-500 text-white">
        <Timer className="h-5 w-5" />
      </span>
      <span
        className={`text-lg font-semibold tracking-tight ${light ? "text-white" : "text-ink-900"}`}
      >
        Hourwise
      </span>
    </span>
  );
}
