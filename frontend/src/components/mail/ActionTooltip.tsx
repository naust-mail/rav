"use client";

import { Tooltip } from "radix-ui";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/useUiStore";

type ActionTooltipProps = {
  /** Text shown in the tooltip. */
  label: string;
  children: React.ReactNode;
};

/** Wraps a single action button with a styled Radix tooltip. Use inside ActionTooltip.Provider. */
export function ActionTooltip({ label, children }: ActionTooltipProps) {
  const shouldAnimate = useUiStore((s) => s.effectiveAnimationMode) !== "off";

  return (
    <Tooltip.Root>
      <Tooltip.Trigger asChild>{children}</Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content
          side="bottom"
          sideOffset={6}
          className={cn(
            "z-50 rounded-md border border-border bg-popover px-2.5 py-1.5 text-xs font-medium text-popover-foreground shadow-md",
            shouldAnimate && "animate-in fade-in-0 zoom-in-95",
          )}
        >
          {label}
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

/** Wrap an action bar that contains ActionTooltip children. delayDuration keeps tooltips from flashing on every hover. */
export function ActionTooltipProvider({ children }: { children: React.ReactNode }) {
  return <Tooltip.Provider delayDuration={600}>{children}</Tooltip.Provider>;
}
