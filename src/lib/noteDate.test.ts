import { describe, expect, it } from "vitest";
import { formatDateChip, fromDateTimeLocal, toDateTimeLocal } from "./noteDate";

describe("note date input (#222)", () => {
  it("writes the note's own local wall-clock time into the picker", () => {
    const ts = new Date(2023, 2, 7, 9, 5).getTime();
    expect(toDateTimeLocal(ts)).toBe("2023-03-07T09:05");
  });

  it("reads the picker back to the same instant", () => {
    expect(fromDateTimeLocal("2023-03-07T09:05")).toBe(new Date(2023, 2, 7, 9, 5).getTime());
  });

  it("reads a cleared or malformed picker as nothing", () => {
    expect(fromDateTimeLocal("")).toBeNull();
    expect(fromDateTimeLocal("not a date")).toBeNull();
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
