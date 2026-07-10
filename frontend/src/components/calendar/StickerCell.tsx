"use client";

import { useMemo } from "react";
import type { StickerDef } from "@/types/sticker";

type StickerCellProps = {
  stickerId: string;
  catalog: StickerDef[];
  faded?: boolean;
};

/**
 * Renders a sticker in a calendar day cell.
 * Sits at the bottom-center of the day cell.
 */
export function StickerCell({ stickerId, catalog, faded = false }: StickerCellProps) {
  const def = useMemo(() => catalog.find((s) => s.id === stickerId), [stickerId, catalog]);
  if (!def) return null;

  return (
    // Sticker assets are pre-resized/re-encoded to WebP at their actual max
    // render size (see frontend/scripts/optimize-stickers.mjs) - next/image
    // would add no benefit here (static export has no server to run its
    // optimizer) and animated stickers need src to stay a plain file path.
    // eslint-disable-next-line @next/next/no-img-element
    <img
      src={`/stickers/${def.file}`}
      alt={def.label}
      title={def.label}
      className="pointer-events-none absolute bottom-0.5 left-1/2 h-16 w-16 -translate-x-1/2 object-contain select-none"
      style={{ filter: "drop-shadow(0 0 1.5px rgba(0,0,0,0.18))", imageRendering: "auto", opacity: faded ? 0.35 : 1 }}
      loading="lazy"
    />
  );
}
