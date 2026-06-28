import { renderHook, act } from "@testing-library/react";
import { describe, expect, it, vi, beforeEach } from "vitest";

const { mockState, mockIsMobile } = vi.hoisted(() => ({
  mockState: {
    selectedDate: new Date("2026-06-15"),
    viewMode: "week" as "month" | "week" | "day",
    setDate: vi.fn(),
  },
  mockIsMobile: { value: false },
}));

vi.mock("@/stores/useCalendarStore", () => ({
  useCalendarStore: (selector: (s: typeof mockState) => unknown) => selector(mockState),
}));

vi.mock("@/hooks/useIsMobile", () => ({
  useIsMobile: () => mockIsMobile.value,
}));

import { useCalendarNavigation } from "../useCalendarNavigation";

describe("useCalendarNavigation", () => {
  beforeEach(() => {
    mockState.setDate.mockClear();
    mockState.selectedDate = new Date("2026-06-15");
    mockState.viewMode = "week";
    mockIsMobile.value = false;
  });

  it("goNext steps 7 days on desktop week view", () => {
    const { result } = renderHook(() => useCalendarNavigation());
    act(() => { result.current.goNext(); });

    const called = mockState.setDate.mock.calls[0][0] as Date;
    expect(called.getDate()).toBe(22);
  });

  it("goPrev steps 7 days on desktop week view", () => {
    const { result } = renderHook(() => useCalendarNavigation());
    act(() => { result.current.goPrev(); });

    const called = mockState.setDate.mock.calls[0][0] as Date;
    expect(called.getDate()).toBe(8);
  });

  it("goNext steps 3 days on mobile week view", () => {
    mockIsMobile.value = true;
    const { result } = renderHook(() => useCalendarNavigation());
    act(() => { result.current.goNext(); });

    const called = mockState.setDate.mock.calls[0][0] as Date;
    expect(called.getDate()).toBe(18);
  });

  it("goPrev steps 3 days on mobile week view", () => {
    mockIsMobile.value = true;
    const { result } = renderHook(() => useCalendarNavigation());
    act(() => { result.current.goPrev(); });

    const called = mockState.setDate.mock.calls[0][0] as Date;
    expect(called.getDate()).toBe(12);
  });

  it("goNext steps 1 month on month view", () => {
    mockState.viewMode = "month";
    const { result } = renderHook(() => useCalendarNavigation());
    act(() => { result.current.goNext(); });

    const called = mockState.setDate.mock.calls[0][0] as Date;
    expect(called.getMonth()).toBe(6); // July
  });

  it("goPrev steps 1 month on month view", () => {
    mockState.viewMode = "month";
    const { result } = renderHook(() => useCalendarNavigation());
    act(() => { result.current.goPrev(); });

    const called = mockState.setDate.mock.calls[0][0] as Date;
    expect(called.getMonth()).toBe(4); // May
  });

  it("goNext steps 1 day on day view", () => {
    mockState.viewMode = "day";
    const { result } = renderHook(() => useCalendarNavigation());
    act(() => { result.current.goNext(); });

    const called = mockState.setDate.mock.calls[0][0] as Date;
    expect(called.getDate()).toBe(16);
  });

  it("mobile flag reflects useIsMobile", () => {
    mockIsMobile.value = true;
    const { result } = renderHook(() => useCalendarNavigation());
    expect(result.current.isMobile).toBe(true);
  });
});
