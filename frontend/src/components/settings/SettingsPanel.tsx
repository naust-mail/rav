"use client";

import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { ChevronLeft } from "lucide-react";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/useUiStore";
import { AnimatedDiv } from "@/lib/motion/AnimatedDiv";
import { createFadeSlideVariants } from "@/lib/motion/variants";
import { getMotionTokens } from "@/lib/motion/config";
import { DisplaySettings } from "./DisplaySettings";
import { IdentitySettings } from "./IdentitySettings";
import { NotificationSettings } from "./NotificationSettings";
import { VacationSettings } from "./VacationSettings";
import { FiltersSettings } from "./FiltersSettings";
import { SecuritySettings } from "./SecuritySettings";
import { PgpSettings } from "./PgpSettings";
import { useServerCapability } from "@/hooks/usePgp";

type Tab = "display" | "notifications" | "identities" | "vacation" | "filters" | "security" | "pgp";

const BASE_TABS: { id: Tab; label: string }[] = [
  { id: "display", label: "Display" },
  { id: "notifications", label: "Notifications" },
  { id: "identities", label: "Identities" },
  { id: "vacation", label: "Vacation" },
  { id: "filters", label: "Filters" },
  { id: "security", label: "Security" },
];

export function SettingsPanel() {
  const setViewMode = useUiStore((s) => s.setViewMode);
  const effectiveAnimationMode = useUiStore((s) => s.effectiveAnimationMode);
  const [activeTab, setActiveTab] = useState<Tab>("display");
  const pgpEnabled = useServerCapability("pgp");
  const TABS = pgpEnabled ? [...BASE_TABS, { id: "pgp" as Tab, label: "PGP Keys" }] : BASE_TABS;

  const shouldAnimate = effectiveAnimationMode !== "off";
  const tokens = getMotionTokens(effectiveAnimationMode);
  const contentVariants = createFadeSlideVariants(effectiveAnimationMode, "y");

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
                  ? "text-foreground"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {tab.label}
              {activeTab === tab.id && shouldAnimate && (
                <motion.span
                  layoutId="tab-indicator"
                  className="absolute inset-x-0 bottom-0 h-0.5 bg-primary"
                  transition={{ type: "spring", stiffness: tokens.spring.stiffness, damping: tokens.spring.damping, mass: tokens.spring.mass }}
                />
              )}
              {activeTab === tab.id && !shouldAnimate && (
                <span className="absolute inset-x-0 bottom-0 h-0.5 bg-primary" />
              )}
            </button>
          ))}
        </div>
      </div>
      <div className="flex-1 overflow-y-auto p-6">
        <AnimatePresence mode="wait" initial={false}>
          <AnimatedDiv
            key={activeTab}
            variants={contentVariants}
            initial="initial"
            animate="animate"
            exit="exit"
            className="h-full"
          >
            {activeTab === "display" && <DisplaySettings />}
            {activeTab === "notifications" && <NotificationSettings />}
            {activeTab === "identities" && <IdentitySettings />}
            {activeTab === "vacation" && <VacationSettings />}
            {activeTab === "filters" && <FiltersSettings />}
            {activeTab === "security" && <SecuritySettings />}
            {activeTab === "pgp" && <PgpSettings />}
          </AnimatedDiv>
        </AnimatePresence>
      </div>
    </div>
  );
}
