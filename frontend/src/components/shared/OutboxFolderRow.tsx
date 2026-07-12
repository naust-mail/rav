"use client";

import { Popover } from "radix-ui";
import { Send, X, RotateCcw, AlertTriangle, Clock } from "lucide-react";
import { useOutboxList, useCancelOutbox, useRetryOutbox } from "@/hooks/useCompose";
import type { OutboxEntry } from "@/hooks/useCompose";
import { useUiStore } from "@/stores/useUiStore";
import { cn } from "@/lib/utils";

function OutboxRow({ entry }: { entry: OutboxEntry }) {
  const cancelMutation = useCancelOutbox();
  const retryMutation = useRetryOutbox();
  const recipient = entry.to_addrs[0] ?? "(no recipient)";
  const extra = entry.to_addrs.length - 1;

  return (
    <div className="flex items-start gap-2 border-b border-border px-3 py-2 last:border-b-0">
      <div className="mt-0.5 shrink-0 text-muted-foreground">
        {entry.state === "failed" ? (
          <AlertTriangle className="size-4 text-destructive" />
        ) : (
          <Clock className="size-4" />
        )}
      </div>
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium">
          {entry.subject || "(no subject)"}
        </p>
        <p className="truncate text-xs text-muted-foreground">
          To: {recipient}{extra > 0 ? ` +${extra} more` : ""}
        </p>
        {entry.state === "failed" && entry.fail_reason && (
          <p className="mt-1 truncate text-xs text-destructive" title={entry.fail_reason}>
            {entry.fail_reason}
          </p>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-1">
        {entry.state === "failed" ? (
          <button
            type="button"
            onClick={() => retryMutation.mutate(entry.id)}
            disabled={retryMutation.isPending}
            aria-label="Retry send"
            className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-50"
          >
            <RotateCcw className="size-4" />
          </button>
        ) : null}
        <button
          type="button"
          onClick={() => cancelMutation.mutate(entry.id)}
          disabled={cancelMutation.isPending || entry.state === "sending"}
          aria-label={entry.state === "failed" ? "Discard" : "Cancel send"}
          className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-50"
        >
          <X className="size-4" />
        </button>
      </div>
    </div>
  );
}

/** Row + panel showing outbox entries still scheduled or permanently failed -
 * lets the user cancel a queued send or retry a failed one. Rendered inline
 * in the folder sidebar (like Drafts/Sent), only while non-empty, since it's
 * staged/not-yet-real mail rather than a top-level app. */
export function OutboxFolderRow() {
  const { data } = useOutboxList();
  const entries = data?.entries ?? [];
  const shouldAnimate = useUiStore((s) => s.effectiveAnimationMode) !== "off";

  if (entries.length === 0) return null;

  const failedCount = entries.filter((e) => e.state === "failed").length;

  return (
    <Popover.Root>
      <Popover.Trigger asChild>
        <button
          type="button"
          aria-label={`Outbox (${entries.length} pending)`}
          className="flex w-full items-center gap-2 rounded-md py-2 pl-2 pr-3 text-sm font-medium text-sidebar-foreground transition-colors hover:bg-sidebar-foreground/10"
        >
          <Send className="size-4 shrink-0" />
          <span className="flex-1 truncate text-left">Outbox</span>
          <span
            className={cn(
              "min-w-[20px] rounded-full px-1.5 py-0.5 text-center text-xs font-semibold",
              failedCount > 0 ? "bg-destructive text-white" : "bg-primary text-primary-foreground",
            )}
          >
            {entries.length}
          </span>
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          className={cn(
            "z-50 w-80 rounded-lg border border-border bg-background shadow-lg",
            shouldAnimate && "duration-150 data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95",
          )}
          side="right"
          align="start"
          sideOffset={8}
        >
          <div className="border-b border-border px-3 py-2">
            <p className="text-sm font-medium">Outbox</p>
          </div>
          <div className="max-h-80 overflow-y-auto">
            {entries.map((entry) => (
              <OutboxRow key={entry.id} entry={entry} />
            ))}
          </div>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
