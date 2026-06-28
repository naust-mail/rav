import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import type { CalendarEvent } from "@/types/calendar";

const mockEvent: CalendarEvent = {
  id: "ev-1",
  title: "Test Event",
  description: "",
  location: "",
  start_time: "2026-06-15T10:00:00",
  end_time: "2026-06-15T11:00:00",
  all_day: false,
  recurrence_rule: null,
  attendees: "",
  organizer: "",
  status: "confirmed",
  source: "local",
  source_uid: null,
  meeting_url: null,
  color: "blue",
  reminder_minutes: null,
  created_at: "2026-06-15T09:00:00",
  updated_at: "2026-06-15T09:00:00",
};

import { EventChip } from "../EventChip";

describe("EventChip", () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it("calls onClick on normal click", () => {
    const onClick = vi.fn();
    const onContextMenu = vi.fn();
    render(
      <EventChip event={mockEvent} onClick={onClick} onContextMenu={onContextMenu}>
        Test
      </EventChip>,
    );
    fireEvent.click(screen.getByText("Test"));
    expect(onClick).toHaveBeenCalledTimes(1);
    expect(onContextMenu).not.toHaveBeenCalled();
  });

  it("calls onContextMenu on right-click", () => {
    const onClick = vi.fn();
    const onContextMenu = vi.fn();
    render(
      <EventChip event={mockEvent} onClick={onClick} onContextMenu={onContextMenu}>
        Test
      </EventChip>,
    );
    fireEvent.contextMenu(screen.getByText("Test"));
    expect(onContextMenu).toHaveBeenCalledWith(expect.any(Number), expect.any(Number), mockEvent);
    expect(onClick).not.toHaveBeenCalled();
  });

  it("calls onContextMenu after long press and suppresses the following click", () => {
    const onClick = vi.fn();
    const onContextMenu = vi.fn();
    render(
      <EventChip event={mockEvent} onClick={onClick} onContextMenu={onContextMenu}>
        Test
      </EventChip>,
    );

    const btn = screen.getByText("Test");
    fireEvent.pointerDown(btn, { clientX: 5, clientY: 5 });
    vi.advanceTimersByTime(500);

    expect(onContextMenu).toHaveBeenCalledTimes(1);

    // Click immediately after long press should be suppressed
    fireEvent.click(btn);
    expect(onClick).not.toHaveBeenCalled();
  });

  it("does not suppress click when no long press fired", () => {
    const onClick = vi.fn();
    const onContextMenu = vi.fn();
    render(
      <EventChip event={mockEvent} onClick={onClick} onContextMenu={onContextMenu}>
        Test
      </EventChip>,
    );

    const btn = screen.getByText("Test");
    fireEvent.pointerDown(btn, { clientX: 5, clientY: 5 });
    // Lift before delay
    fireEvent.pointerUp(btn);
    fireEvent.click(btn);

    expect(onClick).toHaveBeenCalledTimes(1);
    expect(onContextMenu).not.toHaveBeenCalled();
  });

  it("renders children and applies className", () => {
    render(
      <EventChip event={mockEvent} onClick={vi.fn()} onContextMenu={vi.fn()} className="custom-class">
        Event Label
      </EventChip>,
    );
    const btn = screen.getByText("Event Label");
    expect(btn.className).toContain("custom-class");
  });
});
