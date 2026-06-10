"use client";

import { PenSquare } from "lucide-react";
import { useIsMobile } from "@/hooks/useIsMobile";
import { useDisplayPreferences } from "@/hooks/useDisplayPreferences";
import { useComposeStore } from "@/stores/useComposeStore";

export function ComposeFab() {
  const isMobile = useIsMobile();
  const { data: prefs } = useDisplayPreferences();
  const openCompose = useComposeStore((s) => s.openCompose);

  if (!isMobile) return null;
  if (prefs?.mobile_compose === "tab") return null;

  return (
    <button
      type="button"
      onClick={openCompose}
      aria-label="Compose"
      className="md:hidden fixed z-30 flex items-center justify-center size-14 rounded-full bg-primary text-primary-foreground shadow-lg"
      style={{ bottom: "calc(env(safe-area-inset-bottom) + 72px)", right: "1rem" }}
    >
      <PenSquare className="size-6" />
    </button>
  );
}
