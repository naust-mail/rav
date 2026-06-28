"use client";

import { useEffect, useMemo, useRef } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence } from "framer-motion";
import { X } from "lucide-react";
import { AnimatedDiv } from "@/lib/motion/AnimatedDiv";
import { createScaleFadeVariants } from "@/lib/motion/variants";
import { useUiStore } from "@/stores/useUiStore";
import { useCalendarStore } from "@/stores/useCalendarStore";
import { getEventColorClasses, formatTime } from "./calendarUtils";
import { EventChip } from "./EventChip";
import { cn } from "@/lib/utils";
import type { CalendarEvent } from "@/types/calendar";

/** State passed to EventListPopover when open. Null = closed. */
export type EventListPopoverState = {
  anchor: DOMRect;
  events: CalendarEvent[];
  title: string;
};

/** Props for EventListPopover. */
type EventListPopoverProps = {
  state: EventListPopoverState | null;
  onClose: () => void;
  timeFormat: string;
  onEventContextMenu?: (x: number, y: number, event: CalendarEvent) => void;
};

const POPOVER_WIDTH = 240;
const POPOVER_MAX_HEIGHT = 320;
const MARGIN = 8;

export function EventListPopover({ state, onClose, timeFormat, onEventContextMenu }: EventListPopoverProps) {
  const selectEvent = useCalendarStore((s) => s.selectEvent);
  const effectiveAnimationMode = useUiStore((s) => s.effectiveAnimationMode);
  const motionProps = useMemo(() => createScaleFadeVariants(effectiveAnimationMode), [effectiveAnimationMode]);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!state) return;
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    function handleClick(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    }
    document.addEventListener("keydown", handleKey);
    document.addEventListener("mousedown", handleClick);
    return () => {
      document.removeEventListener("keydown", handleKey);
      document.removeEventListener("mousedown", handleClick);
    };
  }, [state, onClose]);

  const style = useMemo(() => {
    if (!state) return {};
    const { anchor } = state;
    let left = anchor.left;
    let top = anchor.bottom + MARGIN;
    if (left + POPOVER_WIDTH > window.innerWidth - MARGIN) {
      left = window.innerWidth - POPOVER_WIDTH - MARGIN;
    }
    if (top + POPOVER_MAX_HEIGHT > window.innerHeight - MARGIN) {
      top = anchor.top - POPOVER_MAX_HEIGHT - MARGIN;
    }
    return { left, top, width: POPOVER_WIDTH };
  }, [state]);

  if (typeof document === "undefined") return null;

  return createPortal(
    <AnimatePresence>
      {state && (
        <AnimatedDiv
          ref={ref}
          variants={motionProps}
          initial="initial"
          animate="animate"
          exit="exit"
          className="fixed z-[60] rounded-lg border border-border bg-background shadow-xl"
          style={style}
        >
          <div className="flex items-center justify-between border-b border-border px-3 py-2">
            <span className="truncate text-xs font-semibold text-foreground">{state.title}</span>
            <button
              type="button"
              onClick={onClose}
              className="ml-2 shrink-0 text-muted-foreground hover:text-foreground"
            >
              <X className="size-3.5" />
            </button>
          </div>
          <div className="max-h-72 overflow-y-auto py-1">
            {state.events.map((ev) => {
              const colors = getEventColorClasses(ev.color);
              return (
                <EventChip
                  key={ev.id}
                  event={ev}
                  onClick={() => { selectEvent(ev.id); onClose(); }}
                  onContextMenu={onEventContextMenu ?? (() => {})}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-accent"
                >
                  <div className={cn("size-2 shrink-0 rounded-full", colors.bg)} />
                  <div className="min-w-0">
                    {!ev.all_day && (
                      <div className="text-[10px] text-muted-foreground">
                        {formatTime(ev.start_time, timeFormat)}
                      </div>
                    )}
                    <div className="truncate text-xs font-medium text-foreground">{ev.title}</div>
                  </div>
                </EventChip>
              );
            })}
          </div>
        </AnimatedDiv>
      )}
    </AnimatePresence>,
    document.body,
  );
}
