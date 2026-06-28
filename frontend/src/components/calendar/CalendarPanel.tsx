"use client";

import { useEffect, useMemo, useRef, useCallback } from "react";
import { AnimatePresence, animate } from "framer-motion";
import { AnimatedDiv } from "@/lib/motion/AnimatedDiv";
import { useCalendarStore } from "@/stores/useCalendarStore";
import { useUiStore } from "@/stores/useUiStore";
import { useCalendarSettings } from "@/hooks/useCalendar";
import { useCalendarNavigation } from "@/hooks/useCalendarNavigation";
import { CalendarHeader } from "./CalendarHeader";
import { MonthView } from "./MonthView";
import { WeekView } from "./WeekView";
import { DayView } from "./DayView";
import { EventForm } from "./EventForm";
import { EventDetail } from "./EventDetail";
import { createFadeSlideVariants } from "@/lib/motion/variants";
import { getMotionTokens } from "@/lib/motion/config";

export function CalendarPanel() {
  const viewMode = useCalendarStore((s) => s.viewMode);
  const setViewMode = useCalendarStore((s) => s.setViewMode);
  const selectedDate = useCalendarStore((s) => s.selectedDate);
  const setDate = useCalendarStore((s) => s.setDate);
  const effectiveAnimationMode = useUiStore((s) => s.effectiveAnimationMode);
  const panelTransition = useMemo(() => createFadeSlideVariants(effectiveAnimationMode, "x"), [effectiveAnimationMode]);
  const { data: settings } = useCalendarSettings();
  const { goPrev, goNext } = useCalendarNavigation();

  const weekStartsOn = settings?.week_starts_on ?? 0;
  const timeFormat = settings?.time_format ?? "12h";

  useEffect(() => {
    if (settings?.default_view) {
      const v = settings.default_view as "month" | "week" | "day";
      if (["month", "week", "day"].includes(v)) {
        setViewMode(v);
      }
    }
  }, [settings?.default_view]); // eslint-disable-line react-hooks/exhaustive-deps

  // Ref for the view content area - imperative animation target
  const viewRef = useRef<HTMLDivElement>(null);
  const isAnimating = useRef(false);

  /** Slide content out, run update, slide back in. */
  const animateNav = useCallback(async (direction: 1 | -1, update: () => void) => {
    if (isAnimating.current || !viewRef.current) {
      update();
      return;
    }
    if (effectiveAnimationMode === "off") {
      update();
      return;
    }

    const tokens = getMotionTokens(effectiveAnimationMode);
    const OUT_X = direction > 0 ? -60 : 60;
    const IN_X = direction > 0 ? 60 : -60;

    isAnimating.current = true;
    await animate(viewRef.current, { opacity: 0, x: OUT_X }, {
      duration: tokens.duration.fast,
      ease: [0.4, 0, 1, 1],
    });

    update(); // data updates here, but view is invisible

    // Snap to entry position, then animate in
    animate(viewRef.current, { opacity: 0, x: IN_X }, { duration: 0 });
    await animate(viewRef.current, { opacity: 1, x: 0 }, {
      duration: tokens.duration.normal,
      ease: [0.2, 0, 0, 1],
    });
    isAnimating.current = false;
  }, [effectiveAnimationMode]);

  const handlePrev = useCallback(() => animateNav(-1, goPrev), [animateNav, goPrev]);
  const handleNext = useCallback(() => animateNav(1, goNext), [animateNav, goNext]);
  const handleToday = useCallback(() => {
    const now = new Date();
    const direction = now >= selectedDate ? 1 : -1;
    animateNav(direction, () => setDate(now));
  }, [animateNav, selectedDate, setDate]);

  // Swipe left/right to navigate
  const touchStartX = useRef<number | null>(null);
  const touchStartY = useRef<number | null>(null);
  const handleTouchStart = useCallback((e: React.TouchEvent) => {
    touchStartX.current = e.touches[0].clientX;
    touchStartY.current = e.touches[0].clientY;
  }, []);
  const handleTouchEnd = useCallback((e: React.TouchEvent) => {
    if (touchStartX.current === null || touchStartY.current === null) return;
    const dx = e.changedTouches[0].clientX - touchStartX.current;
    const dy = e.changedTouches[0].clientY - touchStartY.current;
    touchStartX.current = null;
    touchStartY.current = null;
    if (Math.abs(dx) > Math.abs(dy) && Math.abs(dx) > 60) {
      if (dx < 0) handleNext(); else handlePrev();
    }
  }, [handlePrev, handleNext]);

  return (
    <AnimatedDiv
      data-testid="calendar-panel-transition"
      variants={panelTransition}
      initial={panelTransition.initial}
      animate={panelTransition.animate}
      exit={panelTransition.exit}
      className="flex h-full min-w-0 flex-1 flex-col"
      onTouchStart={handleTouchStart}
      onTouchEnd={handleTouchEnd}
    >
      <CalendarHeader onPrev={handlePrev} onNext={handleNext} onToday={handleToday} />

      {/* viewRef: target for imperative date-navigation animation */}
      <div ref={viewRef} className="flex flex-1 overflow-hidden">
        <AnimatePresence mode="wait" initial={false}>
          {viewMode === "month" && (
            <AnimatedDiv
              key="calendar-month-view"
              data-testid="calendar-month-view-transition"
              variants={panelTransition}
              initial={panelTransition.initial}
              animate={panelTransition.animate}
              exit={panelTransition.exit}
              className="flex min-h-0 min-w-0 flex-1"
            >
              <MonthView weekStartsOn={weekStartsOn} timeFormat={timeFormat} />
            </AnimatedDiv>
          )}
          {viewMode === "week" && (
            <AnimatedDiv
              key="calendar-week-view"
              data-testid="calendar-week-view-transition"
              variants={panelTransition}
              initial={panelTransition.initial}
              animate={panelTransition.animate}
              exit={panelTransition.exit}
              className="flex min-h-0 min-w-0 flex-1"
            >
              <WeekView weekStartsOn={weekStartsOn} timeFormat={timeFormat} />
            </AnimatedDiv>
          )}
          {viewMode === "day" && (
            <AnimatedDiv
              key="calendar-day-view"
              data-testid="calendar-day-view-transition"
              variants={panelTransition}
              initial={panelTransition.initial}
              animate={panelTransition.animate}
              exit={panelTransition.exit}
              className="flex min-h-0 min-w-0 flex-1"
            >
              <DayView timeFormat={timeFormat} />
            </AnimatedDiv>
          )}
        </AnimatePresence>
      </div>

      <EventForm />
      <EventDetail timeFormat={timeFormat} />
    </AnimatedDiv>
  );
}
