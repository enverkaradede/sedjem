import { describe, expect, it } from "vitest";

import { formatBytes, formatPercent, formatSeconds } from "./format";

describe("formatBytes", () => {
  it("scales to the right unit", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1_624_555_275)).toBe("1.5 GB");
    expect(formatBytes(574_041_195)).toBe("547 MB");
  });

  it("guards against nonsense input", () => {
    expect(formatBytes(-5)).toBe("0 B");
    expect(formatBytes(Number.NaN)).toBe("0 B");
  });
});

describe("formatPercent", () => {
  it("floors and clamps", () => {
    expect(formatPercent(1, 3)).toBe("33%");
    expect(formatPercent(5, 4)).toBe("100%");
    expect(formatPercent(1, 0)).toBe("0%");
  });
});

describe("formatSeconds", () => {
  it("uses a Turkish decimal comma", () => {
    expect(formatSeconds(1250)).toBe("1,3 sn");
  });
});
