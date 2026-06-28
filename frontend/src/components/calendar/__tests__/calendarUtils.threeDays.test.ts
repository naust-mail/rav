import { describe, expect, it } from "vitest";
import { getThreeDays, getThreeDayRange, formatThreeDayRange } from "../calendarUtils";

describe("getThreeDays", () => {
  it("returns three consecutive days starting from the given date", () => {
    const base = new Date("2026-06-15");
    const days = getThreeDays(base);
    expect(days).toHaveLength(3);
    expect(days[0].getDate()).toBe(15);
    expect(days[1].getDate()).toBe(16);
    expect(days[2].getDate()).toBe(17);
  });

  it("does not mutate the input date", () => {
    const base = new Date("2026-06-15");
    getThreeDays(base);
    expect(base.getDate()).toBe(15);
  });

  it("wraps correctly at month boundary", () => {
    const base = new Date("2026-06-30");
    const days = getThreeDays(base);
    expect(days[0].getDate()).toBe(30);
    expect(days[1].getDate()).toBe(1);
    expect(days[1].getMonth()).toBe(6); // July
    expect(days[2].getDate()).toBe(2);
  });
});

describe("getThreeDayRange", () => {
  it("returns ISO range spanning all three days", () => {
    const base = new Date("2026-06-15");
    const range = getThreeDayRange(base);
    expect(range.start).toBe("2026-06-15T00:00:00");
    expect(range.end).toBe("2026-06-17T23:59:59");
  });
});

describe("formatThreeDayRange", () => {
  it("formats same-month range with shared month name", () => {
    const base = new Date("2026-06-15");
    const label = formatThreeDayRange(base);
    // Expect "Jun 15 - 17" style (exact text depends on locale but days must appear)
    expect(label).toContain("15");
    expect(label).toContain("17");
  });

  it("formats cross-month range with both month names", () => {
    const base = new Date("2026-06-30");
    const label = formatThreeDayRange(base);
    // Should contain both month abbreviations
    expect(label).toMatch(/Jun/i);
    expect(label).toMatch(/Jul/i);
  });
});
