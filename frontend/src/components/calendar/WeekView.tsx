"use client";

import { useMemo, useRef, useEffect, useState } from "react";
import { useCalendarStore } from "@/stores/useCalendarStore";
import { useCalendarEvents } from "@/hooks/useCalendar";
import {
  getWeekDays,
  getWeekViewRange,
  getThreeDays,
  getThreeDayRange,
  getHours,
  layoutEvents,
  getOverflowGroups,
  isSameDay,
  isToday,
  formatTime,
  getEventColorClasses,
} from "./calendarUtils";
import { useIsMobile } from "@/hooks/useIsMobile";
import { EventListPopover, type EventListPopoverState } from "./EventListPopover";
import { EventChip } from "./EventChip";
import { CalendarContextMenu, type CalendarContextMenuState } from "./CalendarContextMenu";
import { cn } from "@/lib/utils";
import type { CalendarEvent } from "@/types/calendar";

type WeekViewProps = {
  weekStartsOn: number;
  timeFormat: string;
};

export function WeekView({ weekStartsOn, timeFormat }: WeekViewProps) {
  const selectedDate = useCalendarStore((s) => s.selectedDate);
  const setDate = useCalendarStore((s) => s.setDate);
  const selectEvent = useCalendarStore((s) => s.selectEvent);
  const openEventForm = useCalendarStore((s) => s.openEventForm);
  const setViewMode = useCalendarStore((s) => s.setViewMode);
  const scrollRef = useRef<HTMLDivElement>(null);
  const WEEK_COL_CAP = 3;
  const isMobile = useIsMobile();

  const days = useMemo(
    () => isMobile ? getThreeDays(selectedDate) : getWeekDays(selectedDate, weekStartsOn),
    [selectedDate, weekStartsOn, isMobile],
  );

  const range = useMemo(
    () => isMobile ? getThreeDayRange(selectedDate) : getWeekViewRange(selectedDate, weekStartsOn),
    [selectedDate, weekStartsOn, isMobile],
  );
  const { data } = useCalendarEvents(range.start, range.end);
  const events = useMemo(() => data?.events ?? [], [data]);

  const hours = getHours();

  // Scroll to 8am on mount
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = 8 * 60; // 8 hours * 60px/hour
    }
  }, []);

  // Live current-time indicator - updates every minute
  const [currentMinutes, setCurrentMinutes] = useState(() => {
    const now = new Date();
    return now.getHours() * 60 + now.getMinutes();
  });
  useEffect(() => {
    const id = setInterval(() => {
      const now = new Date();
      setCurrentMinutes(now.getHours() * 60 + now.getMinutes());
    }, 60_000);
    return () => clearInterval(id);
  }, []);

  // Categorize events into all-day and timed
  const { allDayEvents, timedEventsByDay } = useMemo(() => {
    const allDay: CalendarEvent[] = [];
    const timed = new Map<number, CalendarEvent[]>();

    for (let i = 0; i < days.length; i++) {
      timed.set(i, []);
    }

    for (const event of events) {
      if (event.all_day) {
        allDay.push(event);
      } else {
        const startDate = new Date(event.start_time);
        for (let i = 0; i < 7; i++) {
          if (isSameDay(startDate, days[i])) {
            timed.get(i)!.push(event);
            break;
          }
        }
      }
    }

    return { allDayEvents: allDay, timedEventsByDay: timed };
  }, [events, days]);

  const [overflowPopover, setOverflowPopover] = useState<EventListPopoverState | null>(null);
  const [contextMenu, setContextMenu] = useState<CalendarContextMenuState | null>(null);

  const handleEventContextMenu = (x: number, y: number, ev: CalendarEvent) => {
    setContextMenu({ x, y, type: "event", eventId: ev.id, eventTitle: ev.title });
  };

  const handleTimeSlotClick = (day: Date, hour: number) => {
    const d = new Date(day);
    d.setHours(hour, 0, 0, 0);
    setDate(d);
    openEventForm();
  };

  const handleEventClick = (e: React.MouseEvent, eventId: string) => {
    e.stopPropagation();
    selectEvent(eventId);
  };


  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      {/* Day headers */}
      <div
        className="grid border-b border-border"
        style={{ gridTemplateColumns: `60px repeat(${days.length}, 1fr)` }}
      >
        <div className="border-r border-border" />
        {days.map((day, i) => (
          <button
            key={i}
            type="button"
            onClick={() => {
              setDate(day);
              setViewMode("day");
            }}
            className={cn(
              "border-r border-border px-2 py-2 text-center transition-colors hover:bg-accent",
              isToday(day) && "bg-primary/10",
            )}
          >
            <div className="text-xs text-muted-foreground">
              {day.toLocaleDateString("en-US", { weekday: "short" })}
            </div>
            <div
              className={cn(
                "text-lg font-medium",
                isToday(day) && "text-primary",
              )}
            >
              {day.getDate()}
            </div>
          </button>
        ))}
      </div>

      {/* All-day events row */}
      {allDayEvents.length > 0 && (
        <div
          className="grid border-b border-border"
          style={{ gridTemplateColumns: `60px repeat(${days.length}, 1fr)` }}
        >
          <div className="border-r border-border px-1 py-1 text-[10px] text-muted-foreground">
            All day
          </div>
          {days.map((day, i) => {
            const dayAllDay = allDayEvents.filter((ev) => {
              const start = new Date(ev.start_time);
              const end = new Date(ev.end_time);
              start.setHours(0, 0, 0, 0);
              end.setHours(23, 59, 59, 999);
              const dayStart = new Date(day);
              dayStart.setHours(0, 0, 0, 0);
              return dayStart >= start && dayStart <= end;
            });
            return (
              <div key={i} className="border-r border-border p-0.5">
                {dayAllDay.map((ev) => {
                  const colors = getEventColorClasses(ev.color);
                  return (
                    <button
                      key={ev.id}
                      type="button"
                      onClick={(e) => handleEventClick(e, ev.id)}
                      className={cn(
                        "block w-full truncate rounded px-1 py-0.5 text-left text-[10px]",
                        colors.bg,
                        colors.text,
                      )}
                    >
                      {ev.title}
                    </button>
                  );
                })}
              </div>
            );
          })}
        </div>
      )}

      {/* Time grid */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto">
        <div className="relative grid" style={{ gridTemplateColumns: `60px repeat(${days.length}, 1fr)` }}>
          {/* Time labels */}
          <div className="border-r border-border">
            {hours.map((hour) => (
              <div
                key={hour}
                className="relative h-[60px] border-b border-border pr-2 text-right"
              >
                <span className="absolute -top-2 right-2 text-[10px] text-muted-foreground">
                  {formatTime(
                    `2025-01-01T${String(hour).padStart(2, "0")}:00:00`,
                    timeFormat,
                  )}
                </span>
              </div>
            ))}
          </div>

          {/* Day columns */}
          {days.map((day, dayIdx) => (
            <div key={dayIdx} className="relative border-r border-border">
              {/* Hour slots */}
              {hours.map((hour) => (
                <div
                  key={hour}
                  onClick={() => handleTimeSlotClick(day, hour)}
                  onContextMenu={(e) => { e.preventDefault(); setContextMenu({ x: e.clientX, y: e.clientY, type: "slot", date: day, hour }); }}
                  className="h-[60px] cursor-pointer border-b border-border transition-colors hover:bg-accent/30"
                />
              ))}

              {/* Current time indicator */}
              {isToday(day) && (
                <div
                  className="pointer-events-none absolute left-0 right-0 z-10 border-t-2 border-red-500"
                  style={{ top: `${currentMinutes}px` }}
                >
                  <div className="absolute -left-1 -top-1.5 size-3 rounded-full bg-red-500" />
                </div>
              )}

              {/* Timed events */}
              {(() => {
                const laid = layoutEvents(timedEventsByDay.get(dayIdx) ?? [], day);
                const overflowGroups = getOverflowGroups(laid, WEEK_COL_CAP);
                return (
                  <>
                    {laid.filter((ev) => ev.column < WEEK_COL_CAP).map((event) => {
                      const colors = getEventColorClasses(event.color);
                      const effectiveCols = Math.min(event.totalColumns, WEEK_COL_CAP);
                      const colWidth = 100 / effectiveCols;
                      return (
                        <EventChip
                          key={event.id}
                          event={event}
                          onClick={(e) => handleEventClick(e, event.id)}
                          onContextMenu={handleEventContextMenu}
                          className={cn(
                            "absolute overflow-hidden rounded px-1 py-0.5 text-left text-[10px] leading-tight",
                            colors.bg,
                            colors.text,
                          )}
                          style={{
                            top: `${event.top}px`,
                            height: `${event.height}px`,
                            left: `calc(${event.column * colWidth}% + 2px)`,
                            right: `calc(${(effectiveCols - event.column - 1) * colWidth}% + 2px)`,
                          }}
                          title={event.title}
                        >
                          <div className="truncate font-medium">{event.title}</div>
                          <div className="opacity-80">{formatTime(event.start_time, timeFormat)}</div>
                        </EventChip>
                      );
                    })}
                    {overflowGroups.map((group, i) => (
                      <button
                        key={`overflow-${i}`}
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          setOverflowPopover({
                            anchor: e.currentTarget.getBoundingClientRect(),
                            events: group.all as unknown as CalendarEvent[],
                            title: `${group.all.length} events`,
                          });
                        }}
                        className="absolute right-0.5 z-10 rounded bg-muted px-1 py-0.5 text-[9px] font-medium text-muted-foreground hover:bg-accent hover:text-foreground"
                        style={{ top: `${group.top}px` }}
                      >
                        +{group.hidden.length}
                      </button>
                    ))}
                  </>
                );
              })()}
            </div>
          ))}
        </div>
      </div>

      <EventListPopover
        state={overflowPopover}
        onClose={() => setOverflowPopover(null)}
        timeFormat={timeFormat}
        onEventContextMenu={handleEventContextMenu}
      />
      <CalendarContextMenu
        state={contextMenu}
        onClose={() => setContextMenu(null)}
        timeFormat={timeFormat}
      />
    </div>
  );
}
