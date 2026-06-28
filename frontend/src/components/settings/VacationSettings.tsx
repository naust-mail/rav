"use client";

import { useState } from "react";
import { Loader2 } from "lucide-react";
import { toast } from "sonner";
import { useVacation, useUpdateVacation } from "@/hooks/useVacation";
import type { UpdateVacationResponder } from "@/types/vacation";
import { cn } from "@/lib/utils";

function Toggle({
  label,
  description,
  value,
  onChange,
}: {
  label: string;
  description: string;
  value: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between rounded-lg border border-border p-4">
      <div>
        <div className="text-sm font-medium">{label}</div>
        <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={value}
        onClick={() => onChange(!value)}
        className={cn(
          "relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors",
          value ? "bg-primary" : "bg-muted",
        )}
      >
        <span
          className={cn(
            "pointer-events-none inline-block size-4 rounded-full bg-background shadow-sm transition-transform",
            value ? "translate-x-4" : "translate-x-0",
          )}
        />
      </button>
    </div>
  );
}

export function VacationSettings() {
  const { data: vacation, isLoading } = useVacation();
  const update = useUpdateVacation();

  const [edits, setEdits] = useState<UpdateVacationResponder>({});

  const set = <K extends keyof UpdateVacationResponder>(key: K, value: UpdateVacationResponder[K]) =>
    setEdits((prev) => ({ ...prev, [key]: value }));

  // Derive display values: edits take priority over server data, then defaults.
  const enabled = edits.enabled ?? vacation?.enabled ?? false;
  const subject = edits.subject ?? vacation?.subject ?? "";
  const body = edits.body ?? vacation?.body ?? "";
  const startDate = ("start_date" in edits ? edits.start_date : vacation?.start_date) ?? "";
  const endDate = ("end_date" in edits ? edits.end_date : vacation?.end_date) ?? "";
  const intervalHours = edits.reply_interval_hours ?? vacation?.reply_interval_hours ?? 24;
  const dirty = Object.keys(edits).length > 0;

  const handleSave = () => {
    update.mutate(edits, {
      onSuccess: () => {
        toast.success("Vacation responder saved");
        setEdits({});
      },
      onError: (e) => toast.error(`Failed to save: ${e.message}`),
    });
  };

  if (isLoading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="size-4 animate-spin" />
        Loading vacation settings...
      </div>
    );
  }

  return (
    <div className="max-w-2xl space-y-6">
      <div>
        <h2 className="text-base font-semibold">Vacation Responder</h2>
        <p className="mt-0.5 text-sm text-muted-foreground">
          Automatically reply to incoming messages while you are away.
        </p>
      </div>

      <div className="space-y-3">
        <Toggle
          label="Enable vacation responder"
          description="Send automatic replies to new messages"
          value={enabled}
          onChange={(v) => set("enabled", v)}
        />

        <div className="rounded-lg border border-border p-4 space-y-4">
          <div>
            <label className="block text-sm font-medium mb-1" htmlFor="vac-subject">
              Subject
            </label>
            <input
              id="vac-subject"
              type="text"
              value={subject}
              onChange={(e) => set("subject", e.target.value)}
              placeholder="Out of office: {subject}"
              className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
            />
          </div>

          <div>
            <label className="block text-sm font-medium mb-1" htmlFor="vac-body">
              Message
            </label>
            <textarea
              id="vac-body"
              rows={5}
              value={body}
              onChange={(e) => set("body", e.target.value)}
              placeholder="I am out of the office and will reply when I return."
              className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50 resize-none"
            />
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="block text-sm font-medium mb-1" htmlFor="vac-start">
                Start date (optional)
              </label>
              <input
                id="vac-start"
                type="date"
                value={startDate}
                onChange={(e) => set("start_date", e.target.value || null)}
                className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
              />
            </div>
            <div>
              <label className="block text-sm font-medium mb-1" htmlFor="vac-end">
                End date (optional)
              </label>
              <input
                id="vac-end"
                type="date"
                value={endDate}
                onChange={(e) => set("end_date", e.target.value || null)}
                className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
              />
            </div>
          </div>

          <div>
            <label className="block text-sm font-medium mb-1" htmlFor="vac-interval">
              Reply interval (hours)
            </label>
            <p className="mt-0 mb-1 text-xs text-muted-foreground">
              Minimum time between auto-replies to the same sender.
            </p>
            <input
              id="vac-interval"
              type="number"
              min={1}
              max={168}
              value={intervalHours}
              onChange={(e) => set("reply_interval_hours", Number(e.target.value))}
              className="w-24 rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
            />
          </div>
        </div>
      </div>

      <button
        type="button"
        onClick={handleSave}
        disabled={!dirty || update.isPending}
        className={cn(
          "rounded-md px-4 py-2 text-sm font-medium transition-colors",
          dirty && !update.isPending
            ? "bg-primary text-primary-foreground hover:bg-primary/90"
            : "bg-muted text-muted-foreground cursor-not-allowed",
        )}
      >
        {update.isPending ? "Saving..." : "Save changes"}
      </button>
    </div>
  );
}
