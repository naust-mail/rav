"use client";

import { useState, useCallback, useEffect, useMemo } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence } from "framer-motion";
import { X, Plus, MapPin, AlignLeft, Link, Users, Bell } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { useCalendarStore } from "@/stores/useCalendarStore";
import { useUiStore } from "@/stores/useUiStore";
import {
  useCreateEvent,
  useUpdateEvent,
  useCalendarEvent,
} from "@/hooks/useCalendar";
import {
  EVENT_COLORS,
  REMINDER_OPTIONS,
  formatDateTimeLocal,
} from "./calendarUtils";
import { createFadeSlideVariants, createScaleFadeVariants } from "@/lib/motion/variants";
import { AnimatedDiv } from "@/lib/motion/AnimatedDiv";
import { cn } from "@/lib/utils";

/** Which optional fields are currently visible. */
type OptionalFields = {
  location: boolean;
  description: boolean;
  meetingUrl: boolean;
  attendees: boolean;
  reminder: boolean;
};

/** All editable form fields in one object to allow a single setState call. */
type FormState = {
  title: string;
  description: string;
  location: string;
  startTime: string;
  endTime: string;
  allDay: boolean;
  meetingUrl: string;
  color: string;
  reminderMinutes: number | null;
  attendees: string;
  visible: OptionalFields;
};

const OPTIONAL_FIELD_DEFAULTS: OptionalFields = {
  location: false,
  description: false,
  meetingUrl: false,
  attendees: false,
  reminder: false,
};

const FORM_DEFAULTS: FormState = {
  title: "",
  description: "",
  location: "",
  startTime: "",
  endTime: "",
  allDay: false,
  meetingUrl: "",
  color: "blue",
  reminderMinutes: null,
  attendees: "",
  visible: OPTIONAL_FIELD_DEFAULTS,
};

export function EventForm() {
  const showEventForm = useCalendarStore((s) => s.showEventForm);
  const editingEventId = useCalendarStore((s) => s.editingEventId);
  const closeEventForm = useCalendarStore((s) => s.closeEventForm);
  const selectedDate = useCalendarStore((s) => s.selectedDate);
  const effectiveAnimationMode = useUiStore((s) => s.effectiveAnimationMode);
  const overlayMotionProps = useMemo(() => createFadeSlideVariants(effectiveAnimationMode, "y"), [effectiveAnimationMode]);
  const contentMotionProps = useMemo(() => createScaleFadeVariants(effectiveAnimationMode), [effectiveAnimationMode]);
  const fieldMotionProps = useMemo(() => createScaleFadeVariants(effectiveAnimationMode), [effectiveAnimationMode]);

  const { data: editingEvent } = useCalendarEvent(editingEventId);
  const createEvent = useCreateEvent();
  const updateEvent = useUpdateEvent();

  const isEditing = !!editingEventId;
  const isPending = createEvent.isPending || updateEvent.isPending;

  const [form, setForm] = useState<FormState>(FORM_DEFAULTS);

  const showField = (field: keyof OptionalFields) =>
    setForm((f) => ({ ...f, visible: { ...f.visible, [field]: true } }));

  const hideField = (field: keyof OptionalFields) => {
    setForm((f) => {
      const cleared: Partial<FormState> = { visible: { ...f.visible, [field]: false } };
      if (field === "location") cleared.location = "";
      else if (field === "description") cleared.description = "";
      else if (field === "meetingUrl") cleared.meetingUrl = "";
      else if (field === "attendees") cleared.attendees = "";
      else if (field === "reminder") cleared.reminderMinutes = null;
      return { ...f, ...cleared };
    });
  };

  useEffect(() => {
    if (!showEventForm) return;

    if (editingEvent) {
      let parsedAttendees = "";
      try {
        const parsed = JSON.parse(editingEvent.attendees);
        parsedAttendees = Array.isArray(parsed) ? parsed.join(", ") : "";
      } catch { /* ignore */ }

      let attendeesVisible = false;
      try {
        const p = JSON.parse(editingEvent.attendees);
        attendeesVisible = Array.isArray(p) && p.length > 0;
      } catch { /* ignore */ }

      // eslint-disable-next-line react-hooks/set-state-in-effect
      setForm({
        title: editingEvent.title,
        description: editingEvent.description,
        location: editingEvent.location,
        startTime: editingEvent.all_day
          ? editingEvent.start_time.slice(0, 10)
          : editingEvent.start_time.slice(0, 16),
        endTime: editingEvent.all_day
          ? editingEvent.end_time.slice(0, 10)
          : editingEvent.end_time.slice(0, 16),
        allDay: editingEvent.all_day,
        meetingUrl: editingEvent.meeting_url ?? "",
        color: editingEvent.color ?? "blue",
        reminderMinutes: editingEvent.reminder_minutes,
        attendees: parsedAttendees,
        visible: {
          location: !!editingEvent.location.trim(),
          description: !!editingEvent.description.trim(),
          meetingUrl: !!(editingEvent.meeting_url ?? "").trim(),
          attendees: attendeesVisible,
          reminder: editingEvent.reminder_minutes !== null,
        },
      });
    } else {
      const start = new Date(selectedDate);
      if (start.getHours() === 0 && start.getMinutes() === 0) {
        start.setHours(9, 0, 0, 0);
      }
      const end = new Date(start);
      end.setHours(start.getHours() + 1);

      setForm({
        ...FORM_DEFAULTS,
        startTime: formatDateTimeLocal(start),
        endTime: formatDateTimeLocal(end),
      });
    }
  }, [showEventForm, editingEvent, selectedDate]);

  useEffect(() => {
    if (!showEventForm) return;
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") closeEventForm();
    }
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [showEventForm, closeEventForm]);

  const handleSubmit = useCallback(
    (e: React.FormEvent) => {
      e.preventDefault();
      const { title, description, location, startTime, endTime, allDay, meetingUrl, color, reminderMinutes, attendees } = form;
      if (!title.trim()) return;

      const attendeesList = attendees.split(",").map((a) => a.trim()).filter(Boolean);
      const attendeesJson = JSON.stringify(attendeesList);
      const formattedStart = allDay ? `${startTime}T00:00:00` : `${startTime}:00`;
      const formattedEnd = allDay ? `${endTime}T23:59:59` : `${endTime}:00`;

      if (isEditing && editingEventId) {
        updateEvent.mutate(
          {
            id: editingEventId,
            title: title.trim(),
            description: description.trim(),
            location: location.trim(),
            start_time: formattedStart,
            end_time: formattedEnd,
            all_day: allDay,
            meeting_url: meetingUrl.trim() || undefined,
            color,
            reminder_minutes: reminderMinutes ?? undefined,
            attendees: attendeesJson,
          },
          { onSuccess: () => closeEventForm() },
        );
      } else {
        createEvent.mutate(
          {
            title: title.trim(),
            description: description.trim(),
            location: location.trim(),
            start_time: formattedStart,
            end_time: formattedEnd,
            all_day: allDay,
            meeting_url: meetingUrl.trim() || undefined,
            color,
            reminder_minutes: reminderMinutes ?? undefined,
            attendees: attendeesJson,
          },
          { onSuccess: () => closeEventForm() },
        );
      }
    },
    [form, isEditing, editingEventId, createEvent, updateEvent, closeEventForm],
  );

  const addButtons = [
    { field: "location" as const, label: "Add location", icon: <MapPin className="size-3.5" /> },
    { field: "description" as const, label: "Add description", icon: <AlignLeft className="size-3.5" /> },
    { field: "meetingUrl" as const, label: "Add meeting link", icon: <Link className="size-3.5" /> },
    { field: "attendees" as const, label: "Add attendees", icon: <Users className="size-3.5" /> },
    { field: "reminder" as const, label: "Add reminder", icon: <Bell className="size-3.5" /> },
  ].filter((b) => !form.visible[b.field]);

  if (typeof document === "undefined") return null;

  return createPortal(
    <AnimatePresence>
      {showEventForm && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
          <AnimatedDiv
            data-testid="calendar-event-form-overlay-transition"
            variants={overlayMotionProps}
            initial="initial"
            animate="animate"
            exit="exit"
            className="absolute inset-0 bg-black/50"
            onClick={closeEventForm}
          />

          <AnimatedDiv
            data-testid="calendar-event-form-content-transition"
            variants={contentMotionProps}
            initial="initial"
            animate="animate"
            exit="exit"
            className="relative z-10 w-full max-w-lg max-h-[90vh] overflow-y-auto rounded-lg border border-border bg-background p-4 shadow-xl sm:p-6"
          >
            <div className="mb-4 flex items-center justify-between">
              <h2 className="text-lg font-semibold text-foreground">
                {isEditing ? "Edit Event" : "New Event"}
              </h2>
              <Button variant="ghost" size="icon-sm" onClick={closeEventForm} className="text-muted-foreground">
                <X className="size-4" />
              </Button>
            </div>

            <form onSubmit={handleSubmit} className="space-y-4">
              {/* Title */}
              <Input
                id="event-title"
                value={form.title}
                onChange={(e) => setForm((f) => ({ ...f, title: e.target.value }))}
                placeholder="Event title"
                required
                autoFocus
                maxLength={256}
                aria-label="Event title"
              />

              {/* All day + Color */}
              <div className="flex items-center gap-3">
                <Switch
                  checked={form.allDay}
                  onChange={(v) => setForm((f) => ({ ...f, allDay: v }))}
                  aria-labelledby="event-allday-label"
                />
                <span id="event-allday-label" className="text-sm font-medium text-foreground">
                  All day
                </span>
                <div className="ml-auto flex gap-1.5">
                  {EVENT_COLORS.map((c) => (
                    <button
                      key={c.value}
                      type="button"
                      onClick={() => setForm((f) => ({ ...f, color: c.value }))}
                      className={cn(
                        "size-5 rounded-full transition-all",
                        c.bg,
                        form.color === c.value
                          ? "ring-2 ring-foreground ring-offset-2 ring-offset-background"
                          : "opacity-50 hover:opacity-100",
                      )}
                      title={c.name}
                    />
                  ))}
                </div>
              </div>

              {/* Start / End */}
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <div className="space-y-1.5">
                  <Label htmlFor="event-start">Start</Label>
                  <Input
                    id="event-start"
                    type={form.allDay ? "date" : "datetime-local"}
                    value={form.startTime}
                    onChange={(e) => setForm((f) => ({ ...f, startTime: e.target.value }))}
                    required
                  />
                </div>
                <div className="space-y-1.5">
                  <Label htmlFor="event-end">End</Label>
                  <Input
                    id="event-end"
                    type={form.allDay ? "date" : "datetime-local"}
                    value={form.endTime}
                    onChange={(e) => setForm((f) => ({ ...f, endTime: e.target.value }))}
                    required
                  />
                </div>
              </div>

              {/* Optional fields - shown on demand */}
              <AnimatePresence initial={false}>
                {form.visible.location && (
                  <AnimatedDiv key="field-location" variants={fieldMotionProps} initial="initial" animate="animate" exit="exit">
                    <OptionalField label="Location" onRemove={() => hideField("location")}>
                      <Input
                        id="event-location"
                        value={form.location}
                        onChange={(e) => setForm((f) => ({ ...f, location: e.target.value }))}
                        placeholder="Add location"
                        autoFocus
                        maxLength={256}
                      />
                    </OptionalField>
                  </AnimatedDiv>
                )}
                {form.visible.description && (
                  <AnimatedDiv key="field-description" variants={fieldMotionProps} initial="initial" animate="animate" exit="exit">
                    <OptionalField label="Description" onRemove={() => hideField("description")}>
                      <textarea
                        id="event-description"
                        value={form.description}
                        onChange={(e) => setForm((f) => ({ ...f, description: e.target.value }))}
                        placeholder="Add description..."
                        rows={3}
                        autoFocus
                        className="w-full min-w-0 rounded-md border border-input bg-transparent px-3 py-2 text-base shadow-xs placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px] outline-none resize-none dark:bg-input/30 md:text-sm"
                      />
                    </OptionalField>
                  </AnimatedDiv>
                )}
                {form.visible.meetingUrl && (
                  <AnimatedDiv key="field-meeting-url" variants={fieldMotionProps} initial="initial" animate="animate" exit="exit">
                    <OptionalField label="Meeting link" onRemove={() => hideField("meetingUrl")}>
                      <Input
                        id="event-meeting-url"
                        type="url"
                        value={form.meetingUrl}
                        onChange={(e) => setForm((f) => ({ ...f, meetingUrl: e.target.value }))}
                        placeholder="https://zoom.us/j/..."
                        autoFocus
                      />
                    </OptionalField>
                  </AnimatedDiv>
                )}
                {form.visible.attendees && (
                  <AnimatedDiv key="field-attendees" variants={fieldMotionProps} initial="initial" animate="animate" exit="exit">
                    <OptionalField label="Attendees" onRemove={() => hideField("attendees")}>
                      <Input
                        id="event-attendees"
                        value={form.attendees}
                        onChange={(e) => setForm((f) => ({ ...f, attendees: e.target.value }))}
                        placeholder="alice@example.com, bob@example.com"
                        autoFocus
                      />
                    </OptionalField>
                  </AnimatedDiv>
                )}
                {form.visible.reminder && (
                  <AnimatedDiv key="field-reminder" variants={fieldMotionProps} initial="initial" animate="animate" exit="exit">
                    <OptionalField label="Reminder" onRemove={() => hideField("reminder")}>
                      <select
                        id="event-reminder"
                        value={form.reminderMinutes ?? ""}
                        onChange={(e) => setForm((f) => ({ ...f, reminderMinutes: e.target.value ? Number(e.target.value) : null }))}
                        className="h-9 w-full rounded-md border border-input bg-background px-3 text-base text-foreground shadow-xs outline-none focus:border-ring focus:ring-[3px] focus:ring-ring/50 dark:bg-input/30 md:text-sm"
                      >
                        {REMINDER_OPTIONS.map((opt) => (
                          <option key={opt.label} value={opt.value ?? ""}>{opt.label}</option>
                        ))}
                      </select>
                    </OptionalField>
                  </AnimatedDiv>
                )}
              </AnimatePresence>

              {/* Add field buttons */}
              {addButtons.length > 0 && (
                <div className="flex flex-wrap gap-x-4 gap-y-1.5">
                  {addButtons.map((b) => (
                    <button
                      key={b.field}
                      type="button"
                      onClick={() => showField(b.field)}
                      className="flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
                    >
                      <Plus className="size-3.5" />
                      {b.label}
                    </button>
                  ))}
                </div>
              )}

              {/* Actions */}
              <div className="flex justify-end gap-2 pt-2">
                <Button type="button" variant="outline" onClick={closeEventForm} disabled={isPending}>
                  Cancel
                </Button>
                <Button type="submit" disabled={isPending || !form.title.trim()}>
                  {isPending ? "Saving..." : isEditing ? "Update Event" : "Create Event"}
                </Button>
              </div>
            </form>
          </AnimatedDiv>
        </div>
      )}
    </AnimatePresence>,
    document.body,
  );
}

/** Wraps an optional field with a label row and remove button. */
function OptionalField({
  label,
  onRemove,
  children,
}: {
  label: string;
  onRemove: () => void;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between">
        <Label className="text-xs text-muted-foreground">{label}</Label>
        <button
          type="button"
          onClick={onRemove}
          className="text-muted-foreground transition-colors hover:text-foreground"
          aria-label={`Remove ${label}`}
        >
          <X className="size-3.5" />
        </button>
      </div>
      {children}
    </div>
  );
}
