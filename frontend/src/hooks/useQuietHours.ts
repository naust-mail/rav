"use client";

import { useCallback, useState } from "react";

const STORAGE_KEY = "rav-quiet-hours";

/** Quiet hours preference stored in localStorage (no backend needed). */
export type QuietHoursPrefs = {
  /** Whether quiet hours are active. */
  enabled: boolean;
  /** Start time in HH:MM 24h format, e.g. "22:00". */
  start: string;
  /** End time in HH:MM 24h format, e.g. "08:00". Can be earlier than start for overnight ranges. */
  end: string;
};

const DEFAULT: QuietHoursPrefs = { enabled: false, start: "22:00", end: "08:00" };

function load(): QuietHoursPrefs {
  if (typeof window === "undefined") return DEFAULT;
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT;
    return { ...DEFAULT, ...JSON.parse(raw) };
  } catch {
    return DEFAULT;
  }
}

function save(prefs: QuietHoursPrefs) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(prefs));
}

/** Returns true when the current local time falls within the quiet hours window. */
export function isQuietTime(prefs: QuietHoursPrefs): boolean {
  if (!prefs.enabled) return false;
  const now = new Date();
  const nowM = now.getHours() * 60 + now.getMinutes();
  const [sh, sm] = prefs.start.split(":").map(Number);
  const [eh, em] = prefs.end.split(":").map(Number);
  const startM = sh * 60 + sm;
  const endM = eh * 60 + em;
  // Overnight range (e.g. 22:00 - 08:00)
  if (startM > endM) return nowM >= startM || nowM < endM;
  // Same-day range (e.g. 09:00 - 17:00)
  return nowM >= startM && nowM < endM;
}

export function useQuietHours() {
  const [prefs, setPrefs] = useState<QuietHoursPrefs>(load);

  const update = useCallback((patch: Partial<QuietHoursPrefs>) => {
    setPrefs((prev) => {
      const next = { ...prev, ...patch };
      save(next);
      return next;
    });
  }, []);

  return { prefs, update };
}
