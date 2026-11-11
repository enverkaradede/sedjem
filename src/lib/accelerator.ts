/**
 * Converts keyboard events into accelerator strings understood by the native
 * global-hotkey parser, and formats accelerators for display.
 *
 * Uses `KeyboardEvent.code` (physical key), not `key`: on macOS ⌥+letter
 * produces special characters in `key`, and codes are layout-independent.
 */

export interface KeyInput {
  code: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}

export type AcceleratorResult =
  | { kind: "ok"; accelerator: string }
  | { kind: "pending" }
  | { kind: "invalid"; reason: "needs-modifier" | "unsupported" };

const MODIFIER_CODES = new Set([
  "AltLeft",
  "AltRight",
  "ControlLeft",
  "ControlRight",
  "MetaLeft",
  "MetaRight",
  "ShiftLeft",
  "ShiftRight",
  "CapsLock",
  "Fn",
]);

const NAMED_KEYS = new Set([
  "Backquote",
  "Backslash",
  "BracketLeft",
  "BracketRight",
  "Comma",
  "Equal",
  "Minus",
  "Period",
  "Quote",
  "Semicolon",
  "Slash",
  "Backspace",
  "Enter",
  "Space",
  "Tab",
  "Delete",
  "End",
  "Home",
  "PageDown",
  "PageUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "ArrowUp",
]);

const FUNCTION_KEY = /^F([1-9]|1\d|2[0-4])$/;
const LETTER_KEY = /^Key[A-Z]$/;
const DIGIT_KEY = /^Digit\d$/;
const NUMPAD_KEY = /^Numpad(\d|Add|Decimal|Divide|Enter|Equal|Multiply|Subtract)$/;

function isSupportedKey(code: string): boolean {
  return (
    NAMED_KEYS.has(code) || FUNCTION_KEY.test(code) || LETTER_KEY.test(code) || DIGIT_KEY.test(code) || NUMPAD_KEY.test(code)
  );
}

export function acceleratorFromKeys(input: KeyInput): AcceleratorResult {
  if (MODIFIER_CODES.has(input.code)) {
    return { kind: "pending" };
  }
  if (!isSupportedKey(input.code)) {
    return { kind: "invalid", reason: "unsupported" };
  }
  const modifiers: string[] = [];
  if (input.metaKey) modifiers.push("Super");
  if (input.ctrlKey) modifiers.push("Control");
  if (input.altKey) modifiers.push("Alt");
  if (input.shiftKey) modifiers.push("Shift");

  // Shift alone is not a real modifier for this purpose: Shift+Space is typing.
  const hasRealModifier = input.metaKey || input.ctrlKey || input.altKey;
  if (!hasRealModifier && !FUNCTION_KEY.test(input.code)) {
    return { kind: "invalid", reason: "needs-modifier" };
  }
  return { kind: "ok", accelerator: [...modifiers, input.code].join("+") };
}

const MODIFIER_SYMBOLS: Record<string, string> = {
  SUPER: "⌘",
  CMD: "⌘",
  COMMAND: "⌘",
  CMDORCTRL: "⌘",
  COMMANDORCONTROL: "⌘",
  CMDORCONTROL: "⌘",
  COMMANDORCTRL: "⌘",
  CONTROL: "⌃",
  CTRL: "⌃",
  ALT: "⌥",
  OPTION: "⌥",
  SHIFT: "⇧",
};

const KEY_LABELS: Record<string, string> = {
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Enter: "↩",
  Backspace: "⌫",
  Delete: "⌦",
  Tab: "⇥",
  Backquote: "`",
  Backslash: "\\",
  BracketLeft: "[",
  BracketRight: "]",
  Comma: ",",
  Equal: "=",
  Minus: "-",
  Period: ".",
  Quote: "'",
  Semicolon: ";",
  Slash: "/",
};

function keyLabel(token: string): string {
  if (LETTER_KEY.test(token)) return token.slice(3);
  if (DIGIT_KEY.test(token)) return token.slice(5);
  if (token.startsWith("Numpad")) return `Num ${token.slice(6)}`;
  return KEY_LABELS[token] ?? token;
}

/** Splits an accelerator into display chips, e.g. `["⌥", "Space"]`. */
export function formatAccelerator(accelerator: string): string[] {
  return accelerator
    .split("+")
    .filter((part) => part.length > 0)
    .map((part) => MODIFIER_SYMBOLS[part.toUpperCase()] ?? keyLabel(part));
}
