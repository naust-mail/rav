"use client";

import { useState } from "react";
import { ChevronLeft } from "lucide-react";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/useUiStore";
import { DisplaySettings } from "./DisplaySettings";
import { IdentitySettings } from "./IdentitySettings";
import { NotificationSettings } from "./NotificationSettings";
import { VacationSettings } from "./VacationSettings";
import { FiltersSettings } from "./FiltersSettings";

type Tab = "display" | "notifications" | "identities" | "vacation" | "filters";

const TABS: { id: Tab; label: string }[] = [
  { id: "display", label: "Display" },
  { id: "notifications", label: "Notifications" },
  { id: "identities", label: "Identities" },
  { id: "vacation", label: "Vacation" },
  { id: "filters", label: "Filters" },
];

export function SettingsPanel() {
  const setViewMode = useUiStore((s) => s.setViewMode);
  const [activeTab, setActiveTab] = useState<Tab>("display");

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      <div className="border-b border-border px-4 md:px-6">
        <div className="flex items-center gap-2 py-2 md:py-4">
          <button
            type="button"
            aria-label="Back"
            onClick={() => setViewMode("mail")}
            className="md:hidden flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent active:bg-accent/70 hover:text-foreground"
          >
            <ChevronLeft className="size-5" />
          </button>
          <h1 className="text-lg font-semibold">Settings</h1>
        </div>
        <div className="flex gap-1" role="tablist">
          {TABS.map((tab) => (
            <button
              key={tab.id}
              role="tab"
              type="button"
              aria-selected={activeTab === tab.id}
              onClick={() => setActiveTab(tab.id)}
              className={cn(
                "relative px-3 py-2 text-sm font-medium transition-colors",
                activeTab === tab.id
                  ? "text-foreground after:absolute after:inset-x-0 after:bottom-0 after:h-0.5 after:bg-primary"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {tab.label}
            </button>
          ))}
        </div>
      </div>
      <div className="flex-1 overflow-y-auto p-6">
        {activeTab === "display" && <DisplaySettings />}
        {activeTab === "notifications" && <NotificationSettings />}
        {activeTab === "identities" && <IdentitySettings />}
        {activeTab === "vacation" && <VacationSettings />}
        {activeTab === "filters" && <FiltersSettings />}
      </div>
    </div>
  );
}
