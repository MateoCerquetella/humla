import { describe, expect, it } from "vitest";
import { formatDateChip, monthGrid, toTimeInput, withTime } from "./noteDate";

describe("monthGrid", () => {
  it("lays a month out in six Monday-first weeks", () => {
    const days = monthGrid(2026, 8); // September 2026 starts on a Tuesday
    expect(days).toHaveLength(42);
    expect(days[0]).toEqual(new Date(2026, 7, 31));
    expect(days[1]).toEqual(new Date(2026, 8, 1));
    expect(days[41]).toEqual(new Date(2026, 9, 11));
  });

  it("starts on the first when the month starts on a Monday", () => {
    expect(monthGrid(2026, 5)[0]).toEqual(new Date(2026, 5, 1)); // June 2026
  });
});

describe("note time", () => {
  it("writes the note's local wall-clock time into the time field", () => {
    expect(toTimeInput(new Date(2023, 2, 7, 9, 5).getTime())).toBe("09:05");
  });

  it("puts a day and a time back together in local time", () => {
    expect(withTime(new Date(2023, 2, 7), "14:30")).toBe(new Date(2023, 2, 7, 14, 30).getTime());
  });

  it("reads a cleared or malformed time as nothing", () => {
    expect(withTime(new Date(2023, 2, 7), "")).toBeNull();
    expect(withTime(new Date(2023, 2, 7), "25:00")).toBeNull();
  });
});

describe("formatDateChip", () => {
  const now = new Date(2026, 9, 20, 15, 0).getTime();

  it("names today and yesterday", () => {
    expect(formatDateChip(new Date(2026, 9, 20, 8, 0).getTime(), now)).toBe("Today");
    expect(formatDateChip(new Date(2026, 9, 19, 23, 0).getTime(), now)).toBe("Yesterday");
  });

  it("leaves the year off this year's dates and on older ones", () => {
    expect(formatDateChip(new Date(2026, 2, 7).getTime(), now)).not.toMatch(/2026/);
    expect(formatDateChip(new Date(2023, 2, 7).getTime(), now)).toMatch(/2023/);
  });
});
