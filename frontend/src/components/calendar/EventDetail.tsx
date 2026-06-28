"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence } from "framer-motion";
import {
  X,
  MapPin,
  Clock,
  Users,
  Video,
  Pencil,
  Trash2,
  ExternalLink,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Chip } from "@/components/ui/Chip";
import { useCalendarStore } from "@/stores/useCalendarStore";
import { useUiStore } from "@/stores/useUiStore";
import { useCalendarEvent, useDeleteEvent } from "@/hooks/useCalendar";
import { formatTime, getEventColorClasses } from "./calendarUtils";
import { createFadeSlideVariants, createScaleFadeVariants } from "@/lib/motion/variants";
import { AnimatedDiv } from "@/lib/motion/AnimatedDiv";
import { cn } from "@/lib/utils";

/** Props for the EventDetail component. */
type EventDetailProps = {
  timeFormat: string;
};

export function EventDetail({ timeFormat }: EventDetailProps) {
  const selectedEvent = useCalendarStore((s) => s.selectedEvent);
  const selectEvent = useCalendarStore((s) => s.selectEvent);
  const openEventForm = useCalendarStore((s) => s.openEventForm);
  const effectiveAnimationMode = useUiStore((s) => s.effectiveAnimationMode);
  const overlayMotionProps = useMemo(() => createFadeSlideVariants(effectiveAnimationMode, "y"), [effectiveAnimationMode]);
  const contentMotionProps = useMemo(() => createScaleFadeVariants(effectiveAnimationMode), [effectiveAnimationMode]);
  const deleteConfirmMotion = useMemo(() => createScaleFadeVariants(effectiveAnimationMode), [effectiveAnimationMode]);

  const { data: event } = useCalendarEvent(selectedEvent);
  const deleteEvent = useDeleteEvent();

  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false);
  const deleteConfirmRef = useRef<HTMLDivElement>(null);

  // Close on Escape
  useEffect(() => {
    if (!selectedEvent) return;
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        if (deleteConfirmOpen) {
          setDeleteConfirmOpen(false);
        } else {
          selectEvent(null);
        }
      }
    }
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [selectedEvent, selectEvent, deleteConfirmOpen]);

  useEffect(() => {
    if (!deleteConfirmOpen) return;
    function handleClick(e: MouseEvent) {
      if (deleteConfirmRef.current && !deleteConfirmRef.current.contains(e.target as Node)) {
        setDeleteConfirmOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [deleteConfirmOpen]);

  const attendees = useMemo(() => {
    if (!event) return [];
    try {
      const parsed = JSON.parse(event.attendees);
      return Array.isArray(parsed) ? parsed : [];
    } catch {
      return [];
    }
  }, [event]);

  const handleEdit = () => {
    if (!event) return;
    selectEvent(null);
    openEventForm(event.id);
  };

  const formatDateRange = () => {
    if (!event) return "";
    if (event.all_day) {
      const start = new Date(event.start_time);
      const end = new Date(event.end_time);
      const startStr = start.toLocaleDateString("en-US", { weekday: "short", month: "short", day: "numeric" });
      const endStr = end.toLocaleDateString("en-US", { weekday: "short", month: "short", day: "numeric" });
      return startStr === endStr ? startStr : `${startStr} - ${endStr}`;
    }
    const start = new Date(event.start_time);
    const dateStr = start.toLocaleDateString("en-US", { weekday: "short", month: "short", day: "numeric" });
    return `${dateStr}, ${formatTime(event.start_time, timeFormat)} - ${formatTime(event.end_time, timeFormat)}`;
  };

  if (typeof document === "undefined") return null;

  const isOpen = !!(selectedEvent && event);
  const colors = event ? getEventColorClasses(event.color) : null;

  return createPortal(
    <AnimatePresence>
      {isOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center">
          <AnimatedDiv
            data-testid="calendar-event-detail-overlay-transition"
            variants={overlayMotionProps}
            initial="initial"
            animate="animate"
            exit="exit"
            className="absolute inset-0 bg-black/50"
            onClick={() => selectEvent(null)}
          />

          <AnimatedDiv
            data-testid="calendar-event-detail-content-transition"
            variants={contentMotionProps}
            initial="initial"
            animate="animate"
            exit="exit"
            className="relative z-10 w-full max-w-md rounded-lg border border-border bg-background shadow-xl"
          >
            {/* Color bar */}
            <div className={cn("h-2 rounded-t-lg", colors?.bg)} />

            <div className="p-5">
              {/* Header */}
              <div className="mb-3 flex items-start justify-between">
                <h2 className="line-clamp-2 pr-8 text-lg font-semibold text-foreground">
                  {event!.title}
                </h2>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  onClick={() => selectEvent(null)}
                  className="shrink-0 text-muted-foreground"
                >
                  <X className="size-4" />
                </Button>
              </div>

              {/* Details */}
              <div className="space-y-3">
                <div className="flex items-center gap-2 text-sm text-foreground">
                  <Clock className="size-4 shrink-0 text-muted-foreground" />
                  <span>{formatDateRange()}</span>
                  {event!.all_day && <Chip variant="muted">All day</Chip>}
                </div>

                {event!.location && (
                  <div className="flex items-start gap-2 text-sm text-foreground">
                    <MapPin className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                    <span className="break-words">{event!.location}</span>
                  </div>
                )}

                {event!.meeting_url && (
                  <div className="flex items-center gap-2 text-sm">
                    <Video className="size-4 shrink-0 text-muted-foreground" />
                    <a
                      href={event!.meeting_url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="inline-flex items-center gap-1 truncate text-primary hover:underline"
                    >
                      Join Meeting
                      <ExternalLink className="size-3 shrink-0" />
                    </a>
                  </div>
                )}

                {attendees.length > 0 && (
                  <div className="flex items-start gap-2 text-sm text-foreground">
                    <Users className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                    <div className="space-y-0.5">
                      {attendees.map((email: string) => (
                        <div key={email} className="break-all text-sm">{email}</div>
                      ))}
                    </div>
                  </div>
                )}

                {event!.organizer && (
                  <div className="truncate text-xs text-muted-foreground">
                    Organizer: {event!.organizer}
                  </div>
                )}

                {event!.description && (
                  <div className="mt-3 max-h-40 overflow-y-auto rounded-md bg-muted/50 p-3 text-sm text-foreground whitespace-pre-wrap">
                    {event!.description}
                  </div>
                )}

                {event!.source !== "manual" && (
                  <div className="text-xs text-muted-foreground">
                    Source: {event!.source}
                  </div>
                )}
              </div>

              {/* Actions */}
              <div className="mt-4 flex justify-end gap-2">
                <Button variant="outline" size="sm" onClick={handleEdit} className="gap-1.5">
                  <Pencil className="size-3.5" />
                  Edit
                </Button>
                <div ref={deleteConfirmRef} className="relative">
                  <Button
                    variant="destructive"
                    size="sm"
                    onClick={() => setDeleteConfirmOpen((v) => !v)}
                    className="gap-1.5"
                  >
                    <Trash2 className="size-3.5" />
                    Delete
                  </Button>
                  <AnimatePresence>
                    {deleteConfirmOpen && (
                      <AnimatedDiv
                        variants={deleteConfirmMotion}
                        initial="initial"
                        animate="animate"
                        exit="exit"
                        className="absolute bottom-full right-0 z-20 mb-1 w-52 rounded-lg border border-border bg-background p-3 shadow-lg"
                      >
                        <p className="mb-3 text-sm font-medium text-foreground">Delete this event?</p>
                        <div className="flex justify-end gap-2">
                          <Button variant="outline" size="sm" onClick={() => setDeleteConfirmOpen(false)}>
                            Cancel
                          </Button>
                          <Button
                            variant="destructive"
                            size="sm"
                            disabled={deleteEvent.isPending}
                            onClick={() => {
                              deleteEvent.mutate(event!.id, { onSuccess: () => selectEvent(null) });
                              setDeleteConfirmOpen(false);
                            }}
                          >
                            {deleteEvent.isPending ? "Deleting..." : "Delete"}
                          </Button>
                        </div>
                      </AnimatedDiv>
                    )}
                  </AnimatePresence>
                </div>
              </div>
            </div>
          </AnimatedDiv>
        </div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
