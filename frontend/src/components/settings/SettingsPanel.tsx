"use client";

import { ChevronLeft } from "lucide-react";
import { useUiStore } from "@/stores/useUiStore";
import { DisplaySettings } from "./DisplaySettings";
import { IdentitySettings } from "./IdentitySettings";
import { NotificationSettings } from "./NotificationSettings";

export function SettingsPanel() {
  const setViewMode = useUiStore((s) => s.setViewMode);

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      <div className="border-b border-border px-4 py-2 md:px-6 md:py-4 flex items-center gap-2">
        <button
          type="button"
          aria-label="Back"
          onClick={() => setViewMode("mail")}
          className="md:hidden flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <ChevronLeft className="size-5" />
        </button>
        <h1 className="text-lg font-semibold">Settings</h1>
      </div>
      <div className="flex-1 space-y-10 overflow-y-auto p-6">
        <DisplaySettings />
        <NotificationSettings />
        <IdentitySettings />
      </div>
    </div>
  );
}
