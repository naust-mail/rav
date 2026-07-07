"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence } from "framer-motion";
import { Pencil, Trash2, Plus, Smile } from "lucide-react";
import { AnimatedDiv } from "@/lib/motion/AnimatedDiv";
import { createScaleFadeVariants } from "@/lib/motion/variants";
import { useUiStore } from "@/stores/useUiStore";
import { useCalendarStore } from "@/stores/useCalendarStore";
import { useDeleteEvent } from "@/hooks/useCalendar";
import { formatTime, formatDateISO } from "./calendarUtils";

/**
 * Context menu state. Null = closed.
 * - "event": right-click/long-press on an event chip
 * - "slot": right-click on a timed hour slot
 * - "day": right-click on an all-day/month cell
 */
export type CalendarContextMenuState =
  | { x: number; y: number; type: "event"; eventId: string; eventTitle: string }
  | { x: number; y: number; type: "slot"; date: Date; hour: number }
  | { x: number; y: number; type: "day"; date: Date };

/** Props for CalendarContextMenu. */
type CalendarContextMenuProps = {
  state: CalendarContextMenuState | null;
  onClose: () => void;
  timeFormat: string;
  /** When provided, shows "Add sticker" on day cells. */
  onAddSticker?: (date: Date) => void;
};

const MENU_WIDTH = 188;
const MENU_MAX_HEIGHT = 160;
const MARGIN = 8;

export function CalendarContextMenu({ state, onClose, timeFormat, onAddSticker }: CalendarContextMenuProps) {
  const openEventForm = useCalendarStore((s) => s.openEventForm);
  const setDate = useCalendarStore((s) => s.setDate);
  const selectEvent = useCalendarStore((s) => s.selectEvent);
  const effectiveAnimationMode = useUiStore((s) => s.effectiveAnimationMode);
  const motionProps = useMemo(() => createScaleFadeVariants(effectiveAnimationMode), [effectiveAnimationMode]);
  const ref = useRef<HTMLDivElement>(null);
  const deleteEvent = useDeleteEvent();
  const [confirmDelete, setConfirmDelete] = useState(false);

  const handleClose = useCallback(() => {
    setConfirmDelete(false);
    onClose();
  }, [onClose]);

  useEffect(() => {
    if (!state) return;
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        if (confirmDelete) { setConfirmDelete(false); return; }
        handleClose();
      }
    }
    function handleClick(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) handleClose();
    }
    document.addEventListener("keydown", handleKey);
    document.addEventListener("mousedown", handleClick);
    return () => {
      document.removeEventListener("keydown", handleKey);
      document.removeEventListener("mousedown", handleClick);
    };
  }, [state, handleClose, confirmDelete]);

  const style = useMemo(() => {
    if (!state) return {};
    let left = state.x + MARGIN;
    let top = state.y + MARGIN;
    if (left + MENU_WIDTH > window.innerWidth - MARGIN) left = state.x - MENU_WIDTH - MARGIN;
    if (top + MENU_MAX_HEIGHT > window.innerHeight - MARGIN) top = state.y - MENU_MAX_HEIGHT - MARGIN;
    return { left, top, width: MENU_WIDTH };
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
          className="fixed z-[70] overflow-hidden rounded-lg border border-border bg-background py-1 shadow-xl"
          style={style}
        >
          {state.type === "event" && !confirmDelete && (
            <>
              <button
                type="button"
                onClick={() => { openEventForm(state.eventId); handleClose(); }}
                className="flex w-full items-center gap-2.5 px-3 py-1.5 text-sm text-foreground hover:bg-accent"
              >
                <Pencil className="size-3.5 shrink-0 text-muted-foreground" />
                Edit
              </button>
              <button
                type="button"
                onClick={() => setConfirmDelete(true)}
                className="flex w-full items-center gap-2.5 px-3 py-1.5 text-sm text-destructive hover:bg-destructive/10"
              >
                <Trash2 className="size-3.5 shrink-0" />
                Delete
              </button>
            </>
          )}

          {state.type === "event" && confirmDelete && (
            <div className="px-3 py-2">
              <p className="mb-2 truncate text-xs text-muted-foreground">
                Delete &quot;{state.eventTitle}&quot;?
              </p>
              <div className="flex gap-2">
                <button
                  type="button"
                  onClick={() => setConfirmDelete(false)}
                  className="flex-1 rounded border border-input px-2 py-1 text-xs hover:bg-accent"
                >
                  Cancel
                </button>
                <button
                  type="button"
                  disabled={deleteEvent.isPending}
                  onClick={() => {
                    deleteEvent.mutate(state.eventId, {
                      onSuccess: () => { selectEvent(null); handleClose(); },
                    });
                  }}
                  className="flex-1 rounded bg-destructive px-2 py-1 text-xs text-destructive-foreground hover:bg-destructive/80 disabled:opacity-50"
                >
                  {deleteEvent.isPending ? "Deleting..." : "Delete"}
                </button>
              </div>
            </div>
          )}

          {state.type === "slot" && (
            <button
              type="button"
              onClick={() => {
                const d = new Date(state.date);
                d.setHours(state.hour, 0, 0, 0);
                setDate(d);
                openEventForm();
                handleClose();
              }}
              className="flex w-full items-center gap-2.5 px-3 py-1.5 text-sm text-foreground hover:bg-accent"
            >
              <Plus className="size-3.5 shrink-0 text-muted-foreground" />
              <span className="truncate">
                New event at {formatTime(
                  `${formatDateISO(state.date)}T${String(state.hour).padStart(2, "0")}:00:00`,
                  timeFormat,
                )}
              </span>
            </button>
          )}

          {state.type === "day" && (
            <>
              <button
                type="button"
                onClick={() => {
                  setDate(state.date);
                  openEventForm();
                  handleClose();
                }}
                className="flex w-full items-center gap-2.5 px-3 py-1.5 text-sm text-foreground hover:bg-accent"
              >
                <Plus className="size-3.5 shrink-0 text-muted-foreground" />
                <span className="truncate">
                  New event on {state.date.toLocaleDateString("en-US", { month: "short", day: "numeric" })}
                </span>
              </button>
              {onAddSticker && (
                <button
                  type="button"
                  onClick={() => onAddSticker(state.date)}
                  className="flex w-full items-center gap-2.5 px-3 py-1.5 text-sm text-foreground hover:bg-accent"
                >
                  <Smile className="size-3.5 shrink-0 text-muted-foreground" />
                  <span className="truncate">Add sticker</span>
                </button>
              )}
            </>
          )}
        </AnimatedDiv>
      )}
    </AnimatePresence>,
    document.body,
  );
}
