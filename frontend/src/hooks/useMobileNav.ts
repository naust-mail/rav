"use client";

import { useCallback } from "react";
import { useUiStore } from "@/stores/useUiStore";
import { useIsMobile } from "./useIsMobile";

export function useMobileNav() {
  const isMobile = useIsMobile();
  const setMobilePanelView = useUiStore((s) => s.setMobilePanelView);
  const mobilePanelView = useUiStore((s) => s.mobilePanelView);

  const navigateTo = useCallback(
    (view: "sidebar" | "list" | "reading") => {
      setMobilePanelView(view);
    },
    [setMobilePanelView],
  );

  const goBack = useCallback(() => {
    if (mobilePanelView === "reading") {
      setMobilePanelView("list");
    } else if (mobilePanelView === "list") {
      setMobilePanelView("sidebar");
    }
  }, [mobilePanelView, setMobilePanelView]);

  return { navigateTo, goBack, mobilePanelView, isMobile };
}
