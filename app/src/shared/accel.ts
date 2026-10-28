// KeyboardEvent → global-hotkey accelerator string (FR-45, spec 7.11).
// The Rust parser (global-hotkey) is the source of truth; this produces
// only token spellings it accepts: Ctrl/Alt/Shift/Super (Win) + a key
// name from its parser (KeyA..Z, Digit0..9, F1..F24, named keys).

/** Modifiers in accelerator order, Win last like the parser's own
 * display order ("Super" is the parser's spelling of the Win key). */
const MODIFIER_ORDER = ["Ctrl", "Alt", "Shift", "Super"] as const;

/** Map an event.code that should be captured by its named key. */
const CODE_BY_CODE: Record<string, string> = {
  Space: "Space",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Escape: "Escape",
  Enter: "Enter",
  Tab: "Tab",
};

/** Whether `code` names a key the global-hotkey parser accepts. */
export function isCapturableCode(code: string): boolean {
  if (/^Key[A-Z]$/.test(code)) return true;
  if (/^Digit[0-9]$/.test(code)) return true;
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return true;
  return (
    code === "Space" ||
    code === "ArrowUp" ||
    code === "ArrowDown" ||
    code === "ArrowLeft" ||
    code === "ArrowRight" ||
    code === "Enter" ||
    code === "Tab"
  );
}

/** The global-hotkey key token for a KeyboardEvent, or null when the
 * event is not a capturable key (modifier-only presses, IME, dead keys).
 * `Escape` is returned as a key so the caller can cancel; saving it is
 * rejected by the Rust validator ("Escape is reserved"). */
export function eventKeyToken(event: {
  code: string;
  key: string;
}): string | null {
  const named = CODE_BY_CODE[event.code];
  if (named !== undefined) {
    return named;
  }
  if (/^Key[A-Z]$/.test(event.code)) return event.code.slice(3);
  if (/^Digit[0-9]$/.test(event.code)) return event.code.slice(5);
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(event.code)) return event.code;
  return null;
}

/** Turn a keydown into an accelerator string like "Ctrl+Alt+K", or null
 * when the press is not a capturable binding (caller keeps listening). */
export function eventToAccelerator(event: {
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
  code: string;
  key: string;
}): string | null {
  const key = eventKeyToken(event);
  if (key === null) return null;
  const held: Record<string, boolean> = {
    Ctrl: event.ctrlKey,
    Alt: event.altKey,
    Shift: event.shiftKey,
    Super: event.metaKey,
  };
  const parts: string[] = [];
  for (const modifier of MODIFIER_ORDER) {
    if (held[modifier]) {
      parts.push(modifier);
    }
  }
  parts.push(key);
  return parts.join("+");
}
