"use client";

import { useCallback, useRef } from "react";

type LongPressEvent = React.TouchEvent | React.MouseEvent;

export function useLongPress({
  onLongPress,
  duration = 400,
  moveThreshold = 8,
}: {
  onLongPress: (e: LongPressEvent) => void;
  duration?: number;
  moveThreshold?: number;
}) {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const startPos = useRef<{ x: number; y: number } | null>(null);

  const getPos = (e: LongPressEvent) => {
    const src = "touches" in e ? e.touches[0] : e;
    return { x: src.clientX, y: src.clientY };
  };

  const start = useCallback(
    (e: LongPressEvent) => {
      startPos.current = getPos(e);
      timer.current = setTimeout(() => onLongPress(e), duration);
    },
    [duration, onLongPress],
  );

  const cancel = useCallback(() => {
    if (timer.current) {
      clearTimeout(timer.current);
      timer.current = null;
    }
    startPos.current = null;
  }, []);

  const move = useCallback(
    (e: LongPressEvent) => {
      if (!startPos.current) return;
      const pos = getPos(e);
      const dx = Math.abs(pos.x - startPos.current.x);
      const dy = Math.abs(pos.y - startPos.current.y);
      if (dx > moveThreshold || dy > moveThreshold) cancel();
    },
    [moveThreshold, cancel],
  );

  return {
    onTouchStart: start,
    onTouchEnd: cancel,
    onTouchMove: move,
    onMouseDown: start,
    onMouseUp: cancel,
    onMouseLeave: cancel,
  };
}
