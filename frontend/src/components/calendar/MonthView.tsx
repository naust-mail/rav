"use client";

import { useMemo, useState } from "react";
import { useCalendarStore } from "@/stores/useCalendarStore";
import { useCalendarEvents } from "@/hooks/useCalendar";
import {
  getMonthGrid,
  getDayNames,
  getMonthViewRange,
  isSameDay,
  isToday,
  getEventColorClasses,
  formatTime,
  formatDateISO,
} from "./calendarUtils";
import { EventListPopover, type EventListPopoverState } from "./EventListPopover";
import { FEATURES } from "@/lib/features";
import { useCalendarStickers, usePutSticker, useDeleteSticker } from "@/hooks/useCalendarStickers";
import { StickerCell } from "./StickerCell";
import { StickerPicker } from "./StickerPicker";
import type { StickerDef } from "@/types/sticker";
import { EventChip } from "./EventChip";
import { CalendarContextMenu, type CalendarContextMenuState } from "./CalendarContextMenu";
import { cn } from "@/lib/utils";
import type { CalendarEvent } from "@/types/calendar";

type MonthViewProps = {
  weekStartsOn: number;
  timeFormat: string;
  /** Sticker catalog - only passed when FEATURES.stickers is true. */
  stickerCatalog?: StickerDef[];
};

export function MonthView({ weekStartsOn, timeFormat, stickerCatalog = [] }: MonthViewProps) {
  const selectedDate = useCalendarStore((s) => s.selectedDate);
  const setDate = useCalendarStore((s) => s.setDate);
  const setViewMode = useCalendarStore((s) => s.setViewMode);
  const selectEvent = useCalendarStore((s) => s.selectEvent);
  const openEventForm = useCalendarStore((s) => s.openEventForm);

  const year = selectedDate.getFullYear();
  const month = selectedDate.getMonth();

  const range = useMemo(
    () => getMonthViewRange(year, month, weekStartsOn),
    [year, month, weekStartsOn],
  );
  const { data } = useCalendarEvents(range.start, range.end);
  const events = useMemo(() => data?.events ?? [], [data]);

  const fromDate = useMemo(() => range.start.slice(0, 10), [range.start]);
  const toDate = useMemo(() => range.end.slice(0, 10), [range.end]);
  const { data: stickersByDate } = useCalendarStickers(
    FEATURES.stickers ? fromDate : "",
    FEATURES.stickers ? toDate : "",
  );
  const putSticker = usePutSticker();
  const deleteSticker = useDeleteSticker();
  const [stickerPickerDate, setStickerPickerDate] = useState<Date | null>(null);

  const days = useMemo(
    () => getMonthGrid(year, month, weekStartsOn),
    [year, month, weekStartsOn],
  );
  const dayNames = useMemo(() => getDayNames(weekStartsOn), [weekStartsOn]);

  const eventsByDay = useMemo(() => {
    const map = new Map<string, CalendarEvent[]>();
    for (const event of events) {
      const start = new Date(event.start_time);
      const end = new Date(event.end_time);

      // For multi-day / all-day events, add to each day they span
      const current = new Date(start);
      if (event.all_day) {
        current.setHours(0, 0, 0, 0);
      }
      const endDate = new Date(end);
      while (current <= endDate) {
        const key = `${current.getFullYear()}-${current.getMonth()}-${current.getDate()}`;
        if (!map.has(key)) map.set(key, []);
        map.get(key)!.push(event);
        current.setDate(current.getDate() + 1);
        current.setHours(0, 0, 0, 0);
      }
    }
    return map;
  }, [events]);

  const handleDayClick = (day: Date) => {
    setDate(day);
  };

  const handleDayDoubleClick = (day: Date) => {
    setDate(day);
    openEventForm();
  };

  const handleEventClick = (e: React.MouseEvent, eventId: string) => {
    e.stopPropagation();
    selectEvent(eventId);
  };

  const handleDayLabelClick = (e: React.MouseEvent, day: Date) => {
    e.stopPropagation();
    setDate(day);
    setViewMode("day");
  };

  const [overflowPopover, setOverflowPopover] = useState<EventListPopoverState | null>(null);
  const [contextMenu, setContextMenu] = useState<CalendarContextMenuState | null>(null);

  const handleEventContextMenu = (x: number, y: number, ev: CalendarEvent) => {
    setContextMenu({ x, y, type: "event", eventId: ev.id, eventTitle: ev.title });
  };

  return (
    <div className="flex flex-1 flex-col overflow-auto">
      {/* Day headers */}
      <div className="grid grid-cols-7 border-b border-border">
        {dayNames.map((name) => (
          <div
            key={name}
            className="px-2 py-1.5 text-center text-xs font-medium text-muted-foreground"
          >
            {name}
          </div>
        ))}
      </div>

      {/* Day grid */}
      <div className="grid flex-1 grid-cols-7 grid-rows-6">
        {days.map((day, idx) => {
          const dayKey = `${day.getFullYear()}-${day.getMonth()}-${day.getDate()}`;
          const dayEvents = eventsByDay.get(dayKey) ?? [];
          const isCurrentMonth = day.getMonth() === month;
          const isSelected = isSameDay(day, selectedDate);
          const isTodayDate = isToday(day);

          return (
            <div
              key={idx}
              onClick={() => handleDayClick(day)}
              onDoubleClick={() => handleDayDoubleClick(day)}
              onContextMenu={(e) => {
                e.preventDefault();
                setContextMenu({ x: e.clientX, y: e.clientY, type: "day", date: day });
              }}
              className={cn(
                "relative min-h-0 cursor-pointer border-b border-r border-border p-1 transition-colors hover:bg-accent/50",
                !isCurrentMonth && "bg-muted/30",
                isSelected && "bg-accent",
              )}
            >
              <button
                type="button"
                onClick={(e) => handleDayLabelClick(e, day)}
                className={cn(
                  "mb-0.5 flex size-6 items-center justify-center rounded-full text-xs transition-colors hover:bg-primary hover:text-primary-foreground",
                  isTodayDate && "bg-primary text-primary-foreground font-bold",
                  !isCurrentMonth && "text-muted-foreground/50",
                  isTodayDate && !isCurrentMonth && 'text-black/50 bg-primary/50'
                )}
              >
                {day.getDate()}
              </button>
              {FEATURES.stickers && stickersByDate?.get(formatDateISO(day)) && (
                <StickerCell
                  stickerId={stickersByDate.get(formatDateISO(day))!.sticker_id}
                  catalog={stickerCatalog}
                  faded={!isCurrentMonth}
                />
              )}

              {/* Event chips */}
              <div className="space-y-0.5 overflow-hidden">
                {dayEvents.slice(0, 3).map((event) => {
                  const colors = getEventColorClasses(event.color);
                  return (
                    <EventChip
                      key={event.id}
                      event={event}
                      onClick={(e) => handleEventClick(e, event.id)}
                      onContextMenu={handleEventContextMenu}
                      className={cn(
                        "block w-full truncate rounded px-1 py-0.5 text-left text-[10px] leading-tight",
                        colors.bg,
                        colors.text,
                      )}
                      title={event.title}
                    >
                      {!event.all_day && (
                        <span className="mr-0.5 opacity-80">
                          {formatTime(event.start_time, timeFormat)}
                        </span>
                      )}
                      {event.title}
                    </EventChip>
                  );
                })}
                {dayEvents.length > 3 && (
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      setOverflowPopover({
                        anchor: e.currentTarget.getBoundingClientRect(),
                        events: dayEvents,
                        title: day.toLocaleDateString("en-US", { weekday: "long", month: "long", day: "numeric" }),
                      });
                    }}
                    className="text-[10px] text-muted-foreground hover:text-foreground"
                  >
                    +{dayEvents.length - 3} more
                  </button>
                )}
              </div>
            </div>
          );
        })}
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
        onAddSticker={
          FEATURES.stickers
            ? (date) => { setStickerPickerDate(date); setContextMenu(null); }
            : undefined
        }
      />
      {FEATURES.stickers && (
        <StickerPicker
          open={stickerPickerDate !== null}
          date={stickerPickerDate}
          currentStickerId={
            stickerPickerDate
              ? (stickersByDate?.get(formatDateISO(stickerPickerDate))?.sticker_id ?? null)
              : null
          }
          catalog={stickerCatalog}
          onSelect={(id) => {
            if (stickerPickerDate) {
              putSticker.mutate({ date: formatDateISO(stickerPickerDate), sticker_id: id });
            }
          }}
          onRemove={() => {
            if (stickerPickerDate) {
              deleteSticker.mutate(formatDateISO(stickerPickerDate));
            }
          }}
          onClose={() => setStickerPickerDate(null)}
        />
      )}
    </div>
  );
}
