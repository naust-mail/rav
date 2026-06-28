"use client";

import { useCallback } from "react";
import { useCalendarStore } from "@/stores/useCalendarStore";
import { useIsMobile } from "./useIsMobile";

/** Shared calendar navigation - week steps by 3 on mobile, 7 on desktop. */
export function useCalendarNavigation() {
  const selectedDate = useCalendarStore((s) => s.selectedDate);
  const viewMode = useCalendarStore((s) => s.viewMode);
  const setDate = useCalendarStore((s) => s.setDate);
  const isMobile = useIsMobile();

  const goPrev = useCallback(() => {
    const d = new Date(selectedDate);
    if (viewMode === "month") {
      d.setMonth(d.getMonth() - 1);
    } else if (viewMode === "week") {
      d.setDate(d.getDate() - (isMobile ? 3 : 7));
    } else {
      d.setDate(d.getDate() - 1);
    }
    setDate(d);
  }, [selectedDate, viewMode, isMobile, setDate]);

  const goNext = useCallback(() => {
    const d = new Date(selectedDate);
    if (viewMode === "month") {
      d.setMonth(d.getMonth() + 1);
    } else if (viewMode === "week") {
      d.setDate(d.getDate() + (isMobile ? 3 : 7));
    } else {
      d.setDate(d.getDate() + 1);
    }
    setDate(d);
  }, [selectedDate, viewMode, isMobile, setDate]);

  return { goPrev, goNext, isMobile };
}
