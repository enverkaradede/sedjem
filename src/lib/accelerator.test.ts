import { describe, expect, it } from "vitest";

import { acceleratorFromKeys, formatAccelerator, type KeyInput } from "./accelerator";

const key = (code: string, mods: Partial<KeyInput> = {}): KeyInput => ({
  code,
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  ...mods,
});

describe("acceleratorFromKeys", () => {
  it("builds modifier + key in a stable order", () => {
    expect(acceleratorFromKeys(key("Space", { altKey: true }))).toEqual({ kind: "ok", accelerator: "Alt+Space" });
    expect(acceleratorFromKeys(key("KeyD", { shiftKey: true, ctrlKey: true, metaKey: true }))).toEqual({
      kind: "ok",
      accelerator: "Super+Control+Shift+KeyD",
    });
  });

  it("waits while only modifiers are held", () => {
    expect(acceleratorFromKeys(key("AltLeft", { altKey: true }))).toEqual({ kind: "pending" });
    expect(acceleratorFromKeys(key("MetaRight", { metaKey: true }))).toEqual({ kind: "pending" });
  });

  it("requires a modifier for ordinary keys so typing is never hijacked", () => {
    expect(acceleratorFromKeys(key("KeyA"))).toEqual({ kind: "invalid", reason: "needs-modifier" });
    expect(acceleratorFromKeys(key("Space", { shiftKey: true }))).toEqual({ kind: "invalid", reason: "needs-modifier" });
  });

  it("allows bare function keys", () => {
    expect(acceleratorFromKeys(key("F5"))).toEqual({ kind: "ok", accelerator: "F5" });
    expect(acceleratorFromKeys(key("F19"))).toEqual({ kind: "ok", accelerator: "F19" });
  });

  it("rejects keys the native parser does not know", () => {
    expect(acceleratorFromKeys(key("IntlBackslash", { altKey: true }))).toEqual({ kind: "invalid", reason: "unsupported" });
  });
});

describe("formatAccelerator", () => {
  it("renders macOS symbols", () => {
    expect(formatAccelerator("Alt+Space")).toEqual(["⌥", "Space"]);
    expect(formatAccelerator("Super+Control+Shift+KeyD")).toEqual(["⌘", "⌃", "⇧", "D"]);
    expect(formatAccelerator("CmdOrCtrl+Digit1")).toEqual(["⌘", "1"]);
    expect(formatAccelerator("Alt+ArrowUp")).toEqual(["⌥", "↑"]);
  });
});
