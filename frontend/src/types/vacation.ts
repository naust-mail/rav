/** Vacation auto-responder settings (one per user). */
export type VacationResponder = {
  /** Whether the responder is currently active. */
  enabled: boolean;
  /** Subject line for auto-reply messages. */
  subject: string;
  /** Body text of the auto-reply. */
  body: string;
  /** ISO date (YYYY-MM-DD) when the responder becomes active, or null for no start bound. */
  start_date: string | null;
  /** ISO date (YYYY-MM-DD) when the responder stops, or null for no end bound. */
  end_date: string | null;
  /** Minimum hours between replies to the same sender. */
  reply_interval_hours: number;
};

/** Partial update for vacation responder settings. */
export type UpdateVacationResponder = {
  enabled?: boolean;
  subject?: string;
  body?: string;
  /** Pass null to clear the date; omit to leave unchanged. */
  start_date?: string | null;
  /** Pass null to clear the date; omit to leave unchanged. */
  end_date?: string | null;
  reply_interval_hours?: number;
};
