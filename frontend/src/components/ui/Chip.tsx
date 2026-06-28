"use client";

import { X } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * Chip/pill component used for tags, filter labels, recipient tokens, and toggle filters.
 *
 * Modes:
 * - Label (default): plain span, no interaction
 * - Dismissible: pass `onRemove` to show an X button
 * - Action/toggle: pass `onClick` to render as a button; use `active` for selected state
 *
 * Variants:
 * - default:     primary-tinted background (bg-primary/10)
 * - destructive: red-tinted background (bg-destructive/10)
 * - muted:       neutral muted background; flips to primary when active
 * - outline:     dashed border, no fill (for add/action affordances)
 */
type ChipProps = {
  children: React.ReactNode;
  variant?: "default" | "destructive" | "muted" | "outline";
  /** Renders an X dismiss button. */
  onRemove?: () => void;
  /** Accessible label for the remove button (required for a11y when onRemove is set). */
  removeLabel?: string;
  /** Renders the chip as a <button>. */
  onClick?: () => void;
  /** Active/selected state, only meaningful when onClick is provided. */
  active?: boolean;
  /** Optional leading icon node. */
  icon?: React.ReactNode;
  className?: string;
};

const baseClasses =
  "inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium";

const variantClasses: Record<NonNullable<ChipProps["variant"]>, string> = {
  default:     "bg-primary/10 text-primary",
  destructive: "bg-destructive/10 text-destructive",
  muted:       "bg-muted text-muted-foreground",
  outline:     "border border-dashed border-muted-foreground/30 text-muted-foreground hover:border-primary hover:text-primary",
};

const removeHoverClasses: Record<NonNullable<ChipProps["variant"]>, string> = {
  default:     "hover:bg-primary/20",
  destructive: "hover:bg-destructive/20",
  muted:       "hover:bg-muted-foreground/20",
  outline:     "hover:bg-muted-foreground/20",
};

export function Chip({
  children,
  variant = "default",
  onRemove,
  removeLabel,
  onClick,
  active = false,
  icon,
  className,
}: ChipProps) {
  const colorClasses =
    onClick && active
      ? "bg-primary text-primary-foreground"
      : variantClasses[variant];

  const content = (
    <>
      {icon}
      {children}
      {onRemove && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
          aria-label={removeLabel}
          className={cn(
            "flex size-3.5 items-center justify-center rounded-full transition-colors",
            removeHoverClasses[variant],
          )}
        >
          <X className="size-2.5" />
        </button>
      )}
    </>
  );

  if (onClick) {
    return (
      <button
        type="button"
        onClick={onClick}
        className={cn(baseClasses, colorClasses, "transition-colors", !active && variant === "muted" && "hover:bg-accent", className)}
      >
        {content}
      </button>
    );
  }

  return (
    <span className={cn(baseClasses, colorClasses, className)}>
      {content}
    </span>
  );
}
