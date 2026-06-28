/**
 * Calendar utility functions for date manipulation and formatting.
 */

/** Preset event colors. */
export const EVENT_COLORS = [
  { name: "Blue", value: "blue", bg: "bg-blue-500", text: "text-white" },
  { name: "Red", value: "red", bg: "bg-red-500", text: "text-white" },
  { name: "Green", value: "green", bg: "bg-green-500", text: "text-white" },
  { name: "Yellow", value: "yellow", bg: "bg-yellow-500", text: "text-white" },
  { name: "Purple", value: "purple", bg: "bg-purple-500", text: "text-white" },
  { name: "Pink", value: "pink", bg: "bg-pink-500", text: "text-white" },
  { name: "Orange", value: "orange", bg: "bg-orange-500", text: "text-white" },
  { name: "Teal", value: "teal", bg: "bg-teal-500", text: "text-white" },
] as const;

/** Reminder options in minutes. */
export const REMINDER_OPTIONS = [
  { label: "None", value: null },
  { label: "5 minutes", value: 5 },
  { label: "15 minutes", value: 15 },
  { label: "30 minutes", value: 30 },
  { label: "1 hour", value: 60 },
  { label: "1 day", value: 1440 },
] as const;

/** Get the color classes for an event by its color value. */
export function getEventColorClasses(color: string | null): {
  bg: string;
  text: string;
} {
  const found = EVENT_COLORS.find((c) => c.value === color);
  return found
    ? { bg: found.bg, text: found.text }
    : { bg: "bg-blue-500", text: "text-white" };
}

/** Get days in a month grid (including padding days from prev/next months). */
export function getMonthGrid(
  year: number,
  month: number,
  weekStartsOn: number,
): Date[] {
  const firstDay = new Date(year, month, 1);
  const lastDay = new Date(year, month + 1, 0);

  // Day of week for the first of the month (0=Sun, 6=Sat)
  let startOffset = firstDay.getDay() - weekStartsOn;
  if (startOffset < 0) startOffset += 7;

  const days: Date[] = [];

  // Previous month padding
  for (let i = startOffset - 1; i >= 0; i--) {
    const d = new Date(year, month, -i);
    days.push(d);
  }

  // Current month days
  for (let d = 1; d <= lastDay.getDate(); d++) {
    days.push(new Date(year, month, d));
  }

  // Next month padding to fill the grid (always show 6 rows = 42 cells)
  while (days.length < 42) {
    const nextDay = new Date(year, month + 1, days.length - startOffset - lastDay.getDate() + 1);
    days.push(nextDay);
  }

  return days;
}

/** Get the 7 days of a week that contains the given date. */
export function getWeekDays(date: Date, weekStartsOn: number): Date[] {
  const d = new Date(date);
  let dayOfWeek = d.getDay() - weekStartsOn;
  if (dayOfWeek < 0) dayOfWeek += 7;

  const start = new Date(d);
  start.setDate(d.getDate() - dayOfWeek);

  const days: Date[] = [];
  for (let i = 0; i < 7; i++) {
    const day = new Date(start);
    day.setDate(start.getDate() + i);
    days.push(day);
  }
  return days;
}

/** Get abbreviated day names starting from the given week start. */
export function getDayNames(weekStartsOn: number): string[] {
  const names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  const result: string[] = [];
  for (let i = 0; i < 7; i++) {
    result.push(names[(i + weekStartsOn) % 7]);
  }
  return result;
}

/** Check if two dates are the same calendar day. */
export function isSameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

/** Check if a date is today. */
export function isToday(date: Date): boolean {
  return isSameDay(date, new Date());
}

/** Format a date as YYYY-MM-DD. */
export function formatDateISO(date: Date): string {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, "0");
  const d = String(date.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}`;
}

/** Format a date as YYYY-MM-DDTHH:mm. */
export function formatDateTimeLocal(date: Date): string {
  const y = date.getFullYear();
  const mo = String(date.getMonth() + 1).padStart(2, "0");
  const d = String(date.getDate()).padStart(2, "0");
  const h = String(date.getHours()).padStart(2, "0");
  const mi = String(date.getMinutes()).padStart(2, "0");
  return `${y}-${mo}-${d}T${h}:${mi}`;
}

/** Format time in 12h or 24h format from a datetime string. */
export function formatTime(
  dateStr: string,
  format: string = "12h",
): string {
  const date = new Date(dateStr);
  if (isNaN(date.getTime())) return dateStr;

  const hours = date.getHours();
  const minutes = String(date.getMinutes()).padStart(2, "0");

  if (format === "24h") {
    return `${String(hours).padStart(2, "0")}:${minutes}`;
  }

  const period = hours >= 12 ? "PM" : "AM";
  const h12 = hours % 12 || 12;
  return `${h12}:${minutes} ${period}`;
}

/** Format month and year for header display. */
export function formatMonthYear(date: Date): string {
  return date.toLocaleDateString("en-US", { month: "long", year: "numeric" });
}

/** Format a date for day view header. */
export function formatDayHeader(date: Date): string {
  return date.toLocaleDateString("en-US", {
    weekday: "long",
    month: "long",
    day: "numeric",
    year: "numeric",
  });
}

/** Get the start of the visible range for month view. */
export function getMonthViewRange(
  year: number,
  month: number,
  weekStartsOn: number,
): { start: string; end: string } {
  const grid = getMonthGrid(year, month, weekStartsOn);
  const first = grid[0];
  const last = grid[grid.length - 1];
  return {
    start: `${formatDateISO(first)}T00:00:00`,
    end: `${formatDateISO(last)}T23:59:59`,
  };
}

/** Get the visible range for a week view. */
export function getWeekViewRange(
  date: Date,
  weekStartsOn: number,
): { start: string; end: string } {
  const days = getWeekDays(date, weekStartsOn);
  return {
    start: `${formatDateISO(days[0])}T00:00:00`,
    end: `${formatDateISO(days[6])}T23:59:59`,
  };
}

/** Get the visible range for a day view. */
export function getDayViewRange(date: Date): { start: string; end: string } {
  const iso = formatDateISO(date);
  return {
    start: `${iso}T00:00:00`,
    end: `${iso}T23:59:59`,
  };
}

/** Hours array 0-23 for time grid. */
export function getHours(): number[] {
  return Array.from({ length: 24 }, (_, i) => i);
}

/** An event augmented with layout data from layoutEvents. */
export type LaidOutEvent<T> = T & {
  top: number;
  height: number;
  column: number;
  totalColumns: number;
};

/** A group of overflow events (column >= cap) with all events in their time band. */
export type OverflowGroup<T> = {
  /** Events hidden due to column cap. */
  hidden: LaidOutEvent<T>[];
  /** All events (visible + hidden) overlapping this group's time band - shown in the popover. */
  all: LaidOutEvent<T>[];
  /** Pixel top of the earliest hidden event - where to place the chip. */
  top: number;
};

/**
 * Given laid-out events and a column cap, returns groups of hidden events
 * (column >= cap) for rendering overflow "+N" chips.
 */
export function getOverflowGroups<T extends { start_time: string; end_time: string }>(
  laidOut: LaidOutEvent<T>[],
  cap: number,
): OverflowGroup<T>[] {
  const hidden = laidOut.filter((ev) => ev.column >= cap);
  if (hidden.length === 0) return [];

  // Connected components among hidden events (by pixel overlap)
  const visited = new Set<number>();
  const components: LaidOutEvent<T>[][] = [];

  for (let i = 0; i < hidden.length; i++) {
    if (visited.has(i)) continue;
    const comp: LaidOutEvent<T>[] = [];
    const stack = [i];
    while (stack.length) {
      const idx = stack.pop()!;
      if (visited.has(idx)) continue;
      visited.add(idx);
      comp.push(hidden[idx]);
      for (let j = 0; j < hidden.length; j++) {
        if (!visited.has(j)) {
          const a = hidden[idx], b = hidden[j];
          if (a.top < b.top + b.height && b.top < a.top + a.height) stack.push(j);
        }
      }
    }
    components.push(comp);
  }

  return components.map((comp) => {
    const minTop = Math.min(...comp.map((e) => e.top));
    const maxBottom = Math.max(...comp.map((e) => e.top + e.height));
    const all = laidOut.filter((ev) => ev.top < maxBottom && ev.top + ev.height > minTop);
    return { hidden: comp, all, top: minTop };
  });
}

/**
 * Assigns non-overlapping columns to a set of timed events for a single day.
 * Returns each event augmented with its pixel position, column index, and
 * total number of columns in its overlap group - enough to render side-by-side.
 */
export function layoutEvents<T extends { start_time: string; end_time: string }>(
  events: T[],
  day: Date,
): LaidOutEvent<T>[] {
  if (events.length === 0) return [];

  const positioned: LaidOutEvent<T>[] = events.map((ev) => ({
    ...ev,
    ...getEventPosition(ev.start_time, ev.end_time, day),
    column: 0,
    totalColumns: 1,
  }));

  // Sort by start time; longer events first when tied (more stable layout)
  positioned.sort((a, b) => a.top - b.top || b.height - a.height);

  // Greedy column assignment: columnEnds[i] = bottom pixel of last event in column i
  const columnEnds: number[] = [];
  for (const ev of positioned) {
    const col = columnEnds.findIndex(end => end <= ev.top);
    if (col === -1) {
      ev.column = columnEnds.length;
      columnEnds.push(ev.top + ev.height);
    } else {
      ev.column = col;
      columnEnds[col] = ev.top + ev.height;
    }
  }

  // totalColumns per event = highest column index among all events it overlaps + 1
  for (const ev of positioned) {
    let maxCol = ev.column;
    for (const other of positioned) {
      if (other !== ev && ev.top < other.top + other.height && other.top < ev.top + ev.height) {
        maxCol = Math.max(maxCol, other.column);
      }
    }
    ev.totalColumns = maxCol + 1;
  }

  return positioned;
}

/** Get 3 days starting from the given date (selected day + 2 forward). */
export function getThreeDays(date: Date): Date[] {
  return [0, 1, 2].map((i) => {
    const d = new Date(date);
    d.setDate(date.getDate() + i);
    return d;
  });
}

/** Get the visible range for a 3-day view. */
export function getThreeDayRange(date: Date): { start: string; end: string } {
  const days = getThreeDays(date);
  return {
    start: `${formatDateISO(days[0])}T00:00:00`,
    end: `${formatDateISO(days[2])}T23:59:59`,
  };
}

/** Format a 3-day range for the header (e.g. "Jun 28 - 30" or "Jun 28 - Jul 1"). */
export function formatThreeDayRange(date: Date): string {
  const days = getThreeDays(date);
  const start = days[0].toLocaleDateString("en-US", { month: "short", day: "numeric" });
  if (days[0].getMonth() === days[2].getMonth()) {
    return `${start} - ${days[2].getDate()}`;
  }
  return `${start} - ${days[2].toLocaleDateString("en-US", { month: "short", day: "numeric" })}`;
}

/** Calculate top position and height for an event in a time grid. */
export function getEventPosition(
  startTime: string,
  endTime: string,
  dayStart: Date,
): { top: number; height: number } {
  const start = new Date(startTime);
  const end = new Date(endTime);

  const dayStartMs = new Date(dayStart).setHours(0, 0, 0, 0);
  const dayEndMs = dayStartMs + 24 * 60 * 60 * 1000;

  const effectiveStart = Math.max(start.getTime(), dayStartMs);
  const effectiveEnd = Math.min(end.getTime(), dayEndMs);

  const startMinutes = (effectiveStart - dayStartMs) / (1000 * 60);
  const durationMinutes = (effectiveEnd - effectiveStart) / (1000 * 60);

  // Each hour = 60px
  const pixelsPerMinute = 60 / 60;
  return {
    top: startMinutes * pixelsPerMinute,
    height: Math.max(durationMinutes * pixelsPerMinute, 20),
  };
}
