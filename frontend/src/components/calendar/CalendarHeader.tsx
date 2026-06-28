"use client";

import { useMemo } from "react";
import { ChevronLeft, ChevronRight, Plus, Calendar } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useCalendarStore } from "@/stores/useCalendarStore";
import {
  formatMonthYear,
  formatDayHeader,
  formatThreeDayRange,
} from "./calendarUtils";
import { useIsMobile } from "@/hooks/useIsMobile";
import { cn } from "@/lib/utils";

type CalendarHeaderProps = {
  onPrev: () => void;
  onNext: () => void;
  onToday: () => void;
};

export function CalendarHeader({ onPrev, onNext, onToday }: CalendarHeaderProps) {
  const selectedDate = useCalendarStore((s) => s.selectedDate);
  const viewMode = useCalendarStore((s) => s.viewMode);
  const setViewMode = useCalendarStore((s) => s.setViewMode);
  const openEventForm = useCalendarStore((s) => s.openEventForm);
  const isMobile = useIsMobile();

  const headerText = useMemo(() => {
    if (viewMode === "day") return formatDayHeader(selectedDate);
    if (viewMode === "week" && isMobile) return formatThreeDayRange(selectedDate);
    return formatMonthYear(selectedDate);
  }, [viewMode, selectedDate, isMobile]);

  const viewModes: Array<{ label: string; short: string; value: "month" | "week" | "day" }> = [
    { label: "Month", short: "M", value: "month" },
    { label: "Week", short: "W", value: "week" },
    { label: "Day", short: "D", value: "day" },
  ];

  return (
    <div className="flex items-center gap-2 border-b border-border px-3 py-2 md:px-4 md:py-3">
      {/* Calendar icon - desktop only shows label */}
      <div className="hidden items-center gap-2 md:flex">
        <Calendar className="size-5 text-primary" />
        <h1 className="text-base font-semibold text-foreground">Calendar</h1>
      </div>

      {/* Nav controls */}
      <div className="flex items-center gap-1">
        <Button variant="outline" size="sm" onClick={onToday} className="hidden sm:inline-flex">
          Today
        </Button>
        <Button variant="ghost" size="icon-sm" onClick={onPrev}>
          <ChevronLeft className="size-4" />
        </Button>
        <Button variant="ghost" size="icon-sm" onClick={onNext}>
          <ChevronRight className="size-4" />
        </Button>
      </div>

      <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">{headerText}</span>

      {/* View mode toggle */}
      <div className="flex rounded-md border border-input">
        {viewModes.map((m) => (
          <button
            key={m.value}
            type="button"
            onClick={() => setViewMode(m.value)}
            className={cn(
              "h-8 px-2 text-xs font-medium transition-colors first:rounded-l-md last:rounded-r-md md:px-3",
              viewMode === m.value
                ? "bg-primary text-primary-foreground"
                : "bg-background text-muted-foreground hover:bg-accent",
            )}
          >
            <span className="md:hidden">{m.short}</span>
            <span className="hidden md:inline">{m.label}</span>
          </button>
        ))}
      </div>

      {/* Add event */}
      <Button size="sm" onClick={() => openEventForm()} className="gap-1.5">
        <Plus className="size-4" />
        <span className="hidden sm:inline">Event</span>
      </Button>
    </div>
  );
}
