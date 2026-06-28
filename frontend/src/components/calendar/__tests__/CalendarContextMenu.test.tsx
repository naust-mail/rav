import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi, beforeEach } from "vitest";
import type { ReactNode } from "react";
import type { CalendarContextMenuState } from "../CalendarContextMenu";

vi.mock("framer-motion", () => ({
  AnimatePresence: ({ children }: { children: ReactNode }) => <>{children}</>,
  motion: { div: ({ children, ...props }: React.HTMLAttributes<HTMLDivElement>) => <div {...props}>{children}</div> },
}));

const { mockCalendarState, mockUiState, mockDeleteEvent } = vi.hoisted(() => ({
  mockCalendarState: {
    openEventForm: vi.fn(),
    setDate: vi.fn(),
    selectEvent: vi.fn(),
  },
  mockUiState: {
    effectiveAnimationMode: "off" as "rich" | "medium" | "subtle" | "off",
  },
  mockDeleteEvent: {
    isPending: false,
    mutate: vi.fn(),
  },
}));

vi.mock("@/stores/useCalendarStore", () => ({
  useCalendarStore: (selector: (s: typeof mockCalendarState) => unknown) => selector(mockCalendarState),
}));

vi.mock("@/stores/useUiStore", () => ({
  useUiStore: (selector: (s: typeof mockUiState) => unknown) => selector(mockUiState),
}));

vi.mock("@/hooks/useCalendar", () => ({
  useDeleteEvent: () => mockDeleteEvent,
}));

vi.mock("@/lib/motion/AnimatedDiv", () => ({
  AnimatedDiv: ({ children, ...props }: React.HTMLAttributes<HTMLDivElement> & { variants?: unknown; initial?: unknown; animate?: unknown; exit?: unknown }) => (
    <div {...(props as React.HTMLAttributes<HTMLDivElement>)}>{children}</div>
  ),
}));

import { CalendarContextMenu } from "../CalendarContextMenu";

function makeEventState(overrides?: Partial<Extract<CalendarContextMenuState, { type: "event" }>>): CalendarContextMenuState {
  return { x: 100, y: 100, type: "event", eventId: "ev-1", eventTitle: "My Meeting", ...overrides };
}

describe("CalendarContextMenu", () => {
  beforeEach(() => {
    mockCalendarState.openEventForm.mockClear();
    mockCalendarState.setDate.mockClear();
    mockCalendarState.selectEvent.mockClear();
    mockDeleteEvent.mutate.mockClear();
    mockDeleteEvent.isPending = false;
  });

  it("renders nothing when state is null", () => {
    const { container } = render(<CalendarContextMenu state={null} onClose={vi.fn()} timeFormat="12h" />);
    expect(container.querySelector(".fixed")).toBeNull();
  });

  it("renders Edit and Delete for event type", () => {
    render(<CalendarContextMenu state={makeEventState()} onClose={vi.fn()} timeFormat="12h" />);
    expect(screen.getByText("Edit")).toBeTruthy();
    expect(screen.getByText("Delete")).toBeTruthy();
  });

  it("Edit click opens event form and closes", () => {
    const onClose = vi.fn();
    render(<CalendarContextMenu state={makeEventState()} onClose={onClose} timeFormat="12h" />);
    fireEvent.click(screen.getByText("Edit"));
    expect(mockCalendarState.openEventForm).toHaveBeenCalledWith("ev-1");
    expect(onClose).toHaveBeenCalled();
  });

  it("Delete click shows confirmation step", () => {
    render(<CalendarContextMenu state={makeEventState()} onClose={vi.fn()} timeFormat="12h" />);
    fireEvent.click(screen.getByText("Delete"));
    expect(screen.getByText(/Delete "My Meeting"\?/)).toBeTruthy();
    expect(screen.getByText("Cancel")).toBeTruthy();
  });

  it("Cancel in confirm step returns to main menu", () => {
    render(<CalendarContextMenu state={makeEventState()} onClose={vi.fn()} timeFormat="12h" />);
    fireEvent.click(screen.getByText("Delete"));
    fireEvent.click(screen.getByText("Cancel"));
    expect(screen.getByText("Edit")).toBeTruthy();
  });

  it("Confirm delete calls deleteEvent.mutate", () => {
    render(<CalendarContextMenu state={makeEventState()} onClose={vi.fn()} timeFormat="12h" />);
    fireEvent.click(screen.getByText("Delete"));
    // Two delete buttons - first is the initial "Delete", second is confirm "Delete"
    const deleteButtons = screen.getAllByText("Delete");
    fireEvent.click(deleteButtons[deleteButtons.length - 1]);
    expect(mockDeleteEvent.mutate).toHaveBeenCalledWith("ev-1", expect.any(Object));
  });

  it("Escape in confirm step goes back to main menu without closing", () => {
    const onClose = vi.fn();
    render(<CalendarContextMenu state={makeEventState()} onClose={onClose} timeFormat="12h" />);
    fireEvent.click(screen.getByText("Delete"));
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.getByText("Edit")).toBeTruthy();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("Escape at main menu level calls onClose", () => {
    const onClose = vi.fn();
    render(<CalendarContextMenu state={makeEventState()} onClose={onClose} timeFormat="12h" />);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });

  it("renders slot menu with time label", () => {
    const state: CalendarContextMenuState = { x: 0, y: 0, type: "slot", date: new Date("2026-06-15"), hour: 14 };
    render(<CalendarContextMenu state={state} onClose={vi.fn()} timeFormat="12h" />);
    expect(screen.getByText(/New event at/)).toBeTruthy();
  });

  it("slot click sets date with correct hour and opens form", () => {
    const onClose = vi.fn();
    const state: CalendarContextMenuState = { x: 0, y: 0, type: "slot", date: new Date("2026-06-15"), hour: 9 };
    render(<CalendarContextMenu state={state} onClose={onClose} timeFormat="12h" />);
    fireEvent.click(screen.getByText(/New event at/));
    expect(mockCalendarState.setDate).toHaveBeenCalled();
    const d = mockCalendarState.setDate.mock.calls[0][0] as Date;
    expect(d.getHours()).toBe(9);
    expect(mockCalendarState.openEventForm).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("renders day menu with date label", () => {
    const state: CalendarContextMenuState = { x: 0, y: 0, type: "day", date: new Date("2026-06-15") };
    render(<CalendarContextMenu state={state} onClose={vi.fn()} timeFormat="12h" />);
    expect(screen.getByText(/New event on/)).toBeTruthy();
    expect(screen.getByText(/Jun 15/)).toBeTruthy();
  });
});
