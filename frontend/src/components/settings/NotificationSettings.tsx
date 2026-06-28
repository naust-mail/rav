"use client";

import { useCallback } from "react";
import { Loader2 } from "lucide-react";
import { toast } from "sonner";
import {
  useNotificationPreferences,
  useUpdateNotificationPreferences,
} from "@/hooks/useNotificationPreferences";
import { useNotifications } from "@/hooks/useNotifications";
import { useQuietHours } from "@/hooks/useQuietHours";
import { useFolders } from "@/hooks/useFolders";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";

function Toggle({
  label,
  description,
  checked,
  onChange,
  disabled,
  disabledReason,
}: {
  label: string;
  description: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  disabledReason?: string;
}) {
  return (
    <div className="flex items-center justify-between rounded-lg border border-border p-4">
      <div>
        <div className="text-sm font-medium">{label}</div>
        <p className="mt-0.5 text-xs text-muted-foreground">
          {disabled && disabledReason ? disabledReason : description}
        </p>
      </div>
      <Switch checked={checked} onChange={onChange} disabled={disabled} />
    </div>
  );
}

export function NotificationSettings() {
  const { data: prefs, isLoading } = useNotificationPreferences();
  const updatePrefs = useUpdateNotificationPreferences();
  const { data: foldersData } = useFolders();
  const { permission: browserPermission, requestPermission } = useNotifications();
  const { prefs: quietHours, update: updateQuietHours } = useQuietHours();

  const handleToggle = useCallback(
    (field: "enabled" | "sound", value: boolean) => {
      updatePrefs.mutate(
        { [field]: value },
        { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
      );
    },
    [updatePrefs],
  );

  const handleFolderToggle = useCallback(
    (folderName: string, add: boolean) => {
      if (!prefs) return;
      const next = add
        ? [...prefs.folders, folderName]
        : prefs.folders.filter((f) => f !== folderName);
      updatePrefs.mutate(
        { folders: next },
        { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
      );
    },
    [prefs, updatePrefs],
  );

  if (isLoading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="size-4 animate-spin" />
        Loading notification settings...
      </div>
    );
  }

  if (!prefs) return null;

  const folders = foldersData?.folders ?? [];

  return (
    <div className="max-w-2xl space-y-6">
      <div>
        <h2 className="text-base font-semibold">Notifications</h2>
        <p className="mt-0.5 text-sm text-muted-foreground">
          Configure desktop notification preferences.
        </p>
      </div>

      {browserPermission === "default" && (
        <div className="flex items-center justify-between rounded-lg border border-border p-4">
          <div>
            <div className="text-sm font-medium">Browser permission required</div>
            <p className="mt-0.5 text-xs text-muted-foreground">
              Grant permission to receive desktop notifications.
            </p>
          </div>
          <button
            type="button"
            onClick={requestPermission}
            className="rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90 active:bg-primary/80"
          >
            Allow
          </button>
        </div>
      )}

      {browserPermission === "denied" && (
        <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 p-4 text-sm text-amber-700 dark:text-amber-400">
          Browser notifications are blocked. Enable them in your browser settings to receive desktop alerts.
        </div>
      )}

      <div className="space-y-3">
        <Toggle
          label="Desktop notifications"
          description="Show browser notifications when new emails arrive"
          checked={prefs.enabled}
          onChange={(v) => handleToggle("enabled", v)}
        />
        <Toggle
          label="Notification sound"
          description="Play a sound when a notification is shown"
          checked={prefs.sound}
          onChange={(v) => handleToggle("sound", v)}
          disabled={!prefs.enabled}
          disabledReason="Enable desktop notifications to configure sound"
        />
      </div>

      <div className="rounded-lg border border-border p-4">
        <div className="mb-3">
          <div className="text-sm font-medium">Notify for folders</div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Choose which folders trigger notifications.
          </p>
        </div>
        <div className="divide-y divide-border">
          {folders.map((folder) => {
            const isChecked = prefs.folders.includes(folder.name);
            return (
              <button
                key={folder.name}
                type="button"
                role="checkbox"
                aria-checked={isChecked}
                disabled={!prefs.enabled}
                onClick={() => handleFolderToggle(folder.name, !isChecked)}
                className={cn(
                  "flex w-full items-center gap-3 py-2 text-left text-sm transition-colors",
                  !prefs.enabled ? "cursor-not-allowed opacity-50" : "cursor-pointer",
                )}
              >
                <span className={cn(
                  "flex size-4 shrink-0 items-center justify-center rounded border transition-colors",
                  isChecked
                    ? "border-primary bg-primary text-primary-foreground"
                    : "border-muted-foreground/40 bg-transparent",
                  prefs.enabled && !isChecked && "hover:border-primary",
                )}>
                  {isChecked && (
                    <svg className="size-3" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M2.5 6l2.5 2.5 4.5-5" />
                    </svg>
                  )}
                </span>
                {folder.name}
              </button>
            );
          })}
        </div>
      </div>

      <div>
        <h3 className="mb-2 text-sm font-medium">Quiet hours</h3>
        <p className="mb-3 text-xs text-muted-foreground">
          Suppress desktop notifications during a set time window. In-app toasts are not affected.
        </p>
        <div className="space-y-3">
          <Toggle
            label="Enable quiet hours"
            description="No desktop notifications during the window below"
            checked={quietHours.enabled}
            onChange={(v) => updateQuietHours({ enabled: v })}
            disabled={!prefs.enabled}
          />
          {quietHours.enabled && (
            <div className="flex items-center gap-4 rounded-lg border border-border p-4">
              <div className="flex flex-1 flex-col gap-1">
                <label className="text-xs font-medium text-muted-foreground">From</label>
                <input
                  type="time"
                  value={quietHours.start}
                  onChange={(e) => updateQuietHours({ start: e.target.value })}
                  className="rounded-md border border-border bg-background px-2 py-1 text-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                />
              </div>
              <div className="flex flex-1 flex-col gap-1">
                <label className="text-xs font-medium text-muted-foreground">To</label>
                <input
                  type="time"
                  value={quietHours.end}
                  onChange={(e) => updateQuietHours({ end: e.target.value })}
                  className="rounded-md border border-border bg-background px-2 py-1 text-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                />
              </div>
              <p className="mt-4 self-end text-xs text-muted-foreground">
                {quietHours.start > quietHours.end ? "Overnight" : "Same day"}
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
