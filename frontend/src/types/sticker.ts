/**
 * A sticker definition from the bundled manifest.
 * Matches public/stickers/manifest.json entries.
 */
export type StickerDef = {
  /** Unique stable ID used to store user assignments in the DB. */
  id: string;
  /** Display label (English). */
  label: string;
  /** Category slug for grouping in the picker. */
  category: string;
  /** Filename relative to /stickers/, e.g. "happy-chick.gif" */
  file: string;
  /** True when the file is an animated GIF. */
  animated: boolean;
};

/** A sticker assigned to a specific calendar date. Matches backend CalendarSticker. */
export type CalendarSticker = {
  /** ISO date: YYYY-MM-DD */
  date: string;
  sticker_id: string;
  updated_at: string;
};

/** Response from GET /api/calendar/stickers */
export type CalendarStickersResponse = {
  stickers: CalendarSticker[];
};

/** Request body for PUT /api/calendar/stickers/{date} */
export type PutStickerRequest = {
  sticker_id: string;
};
