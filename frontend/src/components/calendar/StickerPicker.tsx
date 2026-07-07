"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";
import type { StickerDef } from "@/types/sticker";

type StickerPickerProps = {
  open: boolean;
  date: Date | null;
  /** Currently assigned sticker_id for this date, if any. */
  currentStickerId: string | null;
  catalog: StickerDef[];
  onSelect: (stickerId: string) => void;
  onRemove: () => void;
  onClose: () => void;
};

export function StickerPicker({
  open,
  date,
  currentStickerId,
  catalog,
  onSelect,
  onRemove,
  onClose,
}: StickerPickerProps) {
  const backdropRef = useRef<HTMLDivElement>(null);
  const categories = useMemo(() => {
    const seen = new Set<string>();
    const order: string[] = [];
    for (const s of catalog) {
      if (!seen.has(s.category)) { seen.add(s.category); order.push(s.category); }
    }
    return order;
  }, [catalog]);

  const [activeCategory, setActiveCategory] = useState<string>(() => categories[0] ?? "");
  useEffect(() => { if (categories.length) setActiveCategory(categories[0]); }, [categories]);

  const visible = useMemo(
    () => catalog.filter((s) => s.category === activeCategory),
    [catalog, activeCategory],
  );

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) { if (e.key === "Escape") onClose(); }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (typeof document === "undefined" || !open) return null;

  const label = date
    ? date.toLocaleDateString("en-US", { month: "long", day: "numeric" })
    : "";

  return createPortal(
    <div
      ref={backdropRef}
      className="fixed inset-0 z-[80] flex items-end justify-center bg-black/40"
      onMouseDown={(e) => { if (e.target === backdropRef.current) onClose(); }}
    >
      <div className="flex max-h-[70vh] w-full max-w-md flex-col rounded-t-2xl bg-background shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-border">
          <span className="text-sm font-medium text-foreground">
            Sticker for {label}
          </span>
          <button
            type="button"
            onClick={onClose}
            className="rounded-full p-1 text-muted-foreground hover:bg-accent"
          >
            <X className="size-4" />
          </button>
        </div>

        {/* Category tabs */}
        <div className="flex gap-1 overflow-x-auto px-3 pt-2 pb-1 scrollbar-none">
          {categories.map((cat) => (
            <button
              key={cat}
              type="button"
              onClick={() => setActiveCategory(cat)}
              className={cn(
                "shrink-0 rounded-full px-3 py-1 text-xs font-medium capitalize transition-colors",
                cat === activeCategory
                  ? "bg-primary text-primary-foreground"
                  : "bg-muted text-muted-foreground hover:bg-muted/80",
              )}
            >
              {cat}
            </button>
          ))}
        </div>

        {/* Sticker grid */}
        <div className="flex-1 overflow-y-auto px-3 py-2">
          <div className="grid grid-cols-5 gap-2">
            {visible.map((s) => (
              <button
                key={s.id}
                type="button"
                onClick={() => { onSelect(s.id); onClose(); }}
                className={cn(
                  "flex aspect-square items-center justify-center rounded-xl p-1 transition-colors hover:bg-accent",
                  s.id === currentStickerId && "ring-2 ring-primary bg-accent",
                )}
                title={s.label}
              >
                <img
                  src={`/stickers/${s.file}`}
                  alt={s.label}
                  className="h-full w-full object-contain"
                  loading="lazy"
                />
              </button>
            ))}
          </div>
        </div>

        {/* Remove button - only shown if a sticker is already set */}
        {currentStickerId && (
          <div className="border-t border-border px-4 py-2">
            <button
              type="button"
              onClick={() => { onRemove(); onClose(); }}
              className="w-full rounded-lg py-2 text-sm text-destructive hover:bg-destructive/10"
            >
              Remove sticker
            </button>
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}
