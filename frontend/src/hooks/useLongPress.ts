"use client";

import { useCallback, useRef } from "react";

/** Handlers to spread onto an element for long-press detection. */
type LongPressHandlers = {
  onPointerDown: (e: React.PointerEvent) => void;
  onPointerUp: () => void;
  onPointerMove: (e: React.PointerEvent) => void;
  onPointerCancel: () => void;
};

/** Return value of useLongPress. */
export type LongPressResult = {
  handlers: LongPressHandlers;
  /** True between when long press fires and the next click - check this to suppress the click. */
  triggered: React.MutableRefObject<boolean>;
  /** Resets the triggered flag. Call this after consuming a triggered long press. */
  resetTriggered: () => void;
};

/**
 * Fires callback after pointer is held without moving for `delay` ms.
 * Cancels if the pointer moves more than 5px or lifts before the timeout.
 */
export function useLongPress(
  callback: (e: React.PointerEvent) => void,
  delay = 500,
): LongPressResult {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const startPos = useRef<{ x: number; y: number } | null>(null);
  const savedEvent = useRef<React.PointerEvent | null>(null);
  const triggered = useRef(false);

  const resetTriggered = useCallback(() => { triggered.current = false; }, []);

  const cancel = useCallback(() => {
    if (timer.current) {
      clearTimeout(timer.current);
      timer.current = null;
    }
    startPos.current = null;
    savedEvent.current = null;
  }, []);

  const onPointerDown = useCallback((e: React.PointerEvent) => {
    startPos.current = { x: e.clientX, y: e.clientY };
    savedEvent.current = e;
    triggered.current = false;
    timer.current = setTimeout(() => {
      triggered.current = true;
      if (savedEvent.current) callback(savedEvent.current);
      cancel();
    }, delay);
  }, [callback, delay, cancel]);

  const onPointerMove = useCallback((e: React.PointerEvent) => {
    if (!startPos.current) return;
    if (Math.abs(e.clientX - startPos.current.x) > 5 || Math.abs(e.clientY - startPos.current.y) > 5) {
      cancel();
    }
  }, [cancel]);

  return {
    handlers: { onPointerDown, onPointerUp: cancel, onPointerMove, onPointerCancel: cancel },
    triggered,
    resetTriggered,
  };
}
