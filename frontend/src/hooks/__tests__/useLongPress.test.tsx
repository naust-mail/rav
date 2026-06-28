import { renderHook, act } from "@testing-library/react";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { useLongPress } from "../useLongPress";

function makePointerEvent(type: string, x = 0, y = 0): React.PointerEvent {
  return { clientX: x, clientY: y, type } as unknown as React.PointerEvent;
}

describe("useLongPress", () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it("fires callback after delay when pointer stays still", () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useLongPress(cb, 500));

    act(() => { result.current.handlers.onPointerDown(makePointerEvent("pointerdown", 10, 10)); });
    expect(cb).not.toHaveBeenCalled();

    act(() => { vi.advanceTimersByTime(500); });
    expect(cb).toHaveBeenCalledTimes(1);
  });

  it("sets triggered ref to true when it fires", () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useLongPress(cb, 500));

    act(() => { result.current.handlers.onPointerDown(makePointerEvent("pointerdown")); });
    expect(result.current.triggered.current).toBe(false);

    act(() => { vi.advanceTimersByTime(500); });
    expect(result.current.triggered.current).toBe(true);
  });

  it("cancels if pointer lifts before delay", () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useLongPress(cb, 500));

    act(() => { result.current.handlers.onPointerDown(makePointerEvent("pointerdown")); });
    act(() => { result.current.handlers.onPointerUp(); });
    act(() => { vi.advanceTimersByTime(500); });

    expect(cb).not.toHaveBeenCalled();
    expect(result.current.triggered.current).toBe(false);
  });

  it("cancels if pointer moves more than 5px", () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useLongPress(cb, 500));

    act(() => { result.current.handlers.onPointerDown(makePointerEvent("pointerdown", 0, 0)); });
    act(() => { result.current.handlers.onPointerMove(makePointerEvent("pointermove", 6, 0)); });
    act(() => { vi.advanceTimersByTime(500); });

    expect(cb).not.toHaveBeenCalled();
  });

  it("does not cancel on small move within threshold", () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useLongPress(cb, 500));

    act(() => { result.current.handlers.onPointerDown(makePointerEvent("pointerdown", 0, 0)); });
    act(() => { result.current.handlers.onPointerMove(makePointerEvent("pointermove", 4, 4)); });
    act(() => { vi.advanceTimersByTime(500); });

    expect(cb).toHaveBeenCalledTimes(1);
  });

  it("cancels on pointerCancel", () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useLongPress(cb, 500));

    act(() => { result.current.handlers.onPointerDown(makePointerEvent("pointerdown")); });
    act(() => { result.current.handlers.onPointerCancel(); });
    act(() => { vi.advanceTimersByTime(500); });

    expect(cb).not.toHaveBeenCalled();
  });

  it("resets triggered ref to false on next pointerDown", () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useLongPress(cb, 500));

    // First press - fire
    act(() => { result.current.handlers.onPointerDown(makePointerEvent("pointerdown")); });
    act(() => { vi.advanceTimersByTime(500); });
    expect(result.current.triggered.current).toBe(true);

    // Second press - resets triggered
    act(() => { result.current.handlers.onPointerDown(makePointerEvent("pointerdown")); });
    expect(result.current.triggered.current).toBe(false);
  });
});
