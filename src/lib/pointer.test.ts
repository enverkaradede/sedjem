import { describe, expect, it } from "vitest";

import { isPrimaryPress } from "./pointer";

describe("isPrimaryPress", () => {
  it("accepts a plain left click", () => {
    expect(isPrimaryPress({ button: 0, ctrlKey: false })).toBe(true);
  });

  it("accepts Control+click, which macOS reports as a secondary click", () => {
    expect(isPrimaryPress({ button: 2, ctrlKey: true })).toBe(true);
    expect(isPrimaryPress({ button: 0, ctrlKey: true })).toBe(true);
  });

  it("rejects a genuine right click and the middle button", () => {
    expect(isPrimaryPress({ button: 2, ctrlKey: false })).toBe(false);
    expect(isPrimaryPress({ button: 1, ctrlKey: false })).toBe(false);
  });
});
