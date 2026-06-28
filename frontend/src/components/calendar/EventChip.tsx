"use client";

import { useLongPress } from "@/hooks/useLongPress";
import type { CalendarEvent } from "@/types/calendar";

/** Props for EventChip. */
type EventChipProps = {
  event: CalendarEvent;
  onClick: (e: React.MouseEvent) => void;
  onContextMenu: (x: number, y: number, event: CalendarEvent) => void;
  className?: string;
  style?: React.CSSProperties;
  title?: string;
  children: React.ReactNode;
};

/**
 * Renders a calendar event as a button with right-click and long-press support.
 * Long-press fires the same context menu callback as right-click.
 * When a long press fires, the subsequent click event is suppressed.
 */
export function EventChip({
  event,
  onClick,
  onContextMenu,
  className,
  style,
  title,
  children,
}: EventChipProps) {
  const longPress = useLongPress((e) => {
    onContextMenu(e.clientX, e.clientY, event);
  });

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    onContextMenu(e.clientX, e.clientY, event);
  };

  const handleClick = (e: React.MouseEvent) => {
    if (longPress.triggered.current) {
      longPress.resetTriggered();
      return;
    }
    onClick(e);
  };

  return (
    <button
      type="button"
      className={className}
      style={style}
      title={title}
      onClick={handleClick}
      onContextMenu={handleContextMenu}
      {...longPress.handlers}
    >
      {children}
    </button>
  );
}
