"use client";

import { Mail, Calendar, Users, Search, PenSquare } from "lucide-react";
import { useUiStore } from "@/stores/useUiStore";
import { useMobileNav } from "@/hooks/useMobileNav";
import { useDisplayPreferences, parseMobileNavTabs } from "@/hooks/useDisplayPreferences";
import { useComposeStore } from "@/stores/useComposeStore";
import { cn } from "@/lib/utils";

const OPTIONAL_TAB_ORDER = ["calendar", "contacts", "search", "compose"] as const;

type OptionalTab = (typeof OPTIONAL_TAB_ORDER)[number];

function tabIcon(tab: string) {
  switch (tab) {
    case "mail": return <Mail className="size-5" />;
    case "calendar": return <Calendar className="size-5" />;
    case "contacts": return <Users className="size-5" />;
    case "search": return <Search className="size-5" />;
    case "compose": return <PenSquare className="size-5" />;
    default: return null;
  }
}

function tabLabel(tab: string) {
  switch (tab) {
    case "mail": return "Mail";
    case "calendar": return "Calendar";
    case "contacts": return "Contacts";
    case "search": return "Search";
    case "compose": return "Compose";
    default: return tab;
  }
}

export function BottomTabBar() {
  const { data: prefs } = useDisplayPreferences();
  const viewMode = useUiStore((s) => s.viewMode);
  const setViewMode = useUiStore((s) => s.setViewMode);
  const setSearchActive = useUiStore((s) => s.setSearchActive);
  const { navigateTo } = useMobileNav();
  const openCompose = useComposeStore((s) => s.openCompose);

  const enabledTabs = parseMobileNavTabs(prefs?.mobile_nav_tabs);
  const mobileCompose = prefs?.mobile_compose ?? "fab";

  // Build visible tabs: Mail first, then optional tabs in fixed order, Compose only if tab style
  const visibleTabs: string[] = ["mail"];
  for (const tab of OPTIONAL_TAB_ORDER) {
    if (tab === "compose") {
      if (mobileCompose === "tab") visibleTabs.push("compose");
    } else if (enabledTabs.includes(tab as OptionalTab)) {
      visibleTabs.push(tab);
    }
  }

  const handleTab = (tab: string) => {
    switch (tab) {
      case "mail":
        setViewMode("mail");
        navigateTo("list");
        break;
      case "calendar":
        setViewMode("calendar");
        break;
      case "contacts":
        setViewMode("contacts");
        break;
      case "search":
        setViewMode("mail");
        navigateTo("list");
        setSearchActive(true);
        break;
      case "compose":
        openCompose();
        break;
    }
  };

  const isActive = (tab: string) => {
    if (tab === "mail") return viewMode === "mail";
    if (tab === "calendar") return viewMode === "calendar";
    if (tab === "contacts") return viewMode === "contacts";
    // search and compose are never highlighted as active destinations
    return false;
  };

  return (
    <nav
      className="md:hidden fixed bottom-0 left-0 right-0 z-20 flex items-stretch border-t border-border bg-background"
      style={{ paddingBottom: "env(safe-area-inset-bottom)" }}
      aria-label="Bottom navigation"
    >
      {visibleTabs.map((tab) => (
        <button
          key={tab}
          type="button"
          aria-label={tabLabel(tab)}
          onClick={() => handleTab(tab)}
          className={cn(
            "flex flex-1 flex-col items-center justify-center gap-0.5 py-2 text-[10px] font-medium transition-colors",
            isActive(tab)
              ? "text-primary"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {tabIcon(tab)}
          <span>{tabLabel(tab)}</span>
        </button>
      ))}
    </nav>
  );
}
