// Turning a keypress into a shortcut string the backend understands.
//
// The format is Tauri's accelerator syntax — "Alt+Space", "CmdOrCtrl+Shift+M" —
// and `hotkey.rs` is what validates it. Nothing here is a security boundary: the
// backend re-validates whatever arrives, because a shortcut is a system-wide
// claim and a bare key would swallow that key in every application on the machine.

/** Modifier keys, which cannot be a shortcut on their own. */
const MODIFIER_CODES = new Set([
  "ShiftLeft",
  "ShiftRight",
  "ControlLeft",
  "ControlRight",
  "AltLeft",
  "AltRight",
  "MetaLeft",
  "MetaRight",
  "CapsLock",
]);

/** `event.code` values that map to an accelerator key under a different name. */
const RENAMED: Record<string, string> = {
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Escape: "Escape",
  NumpadEnter: "Enter",
};

/** `event.code` values that pass through unchanged. */
const VERBATIM = new Set([
  "Space",
  "Enter",
  "Tab",
  "Backspace",
  "Delete",
  "Home",
  "End",
  "PageUp",
  "PageDown",
  "Comma",
  "Period",
  "Slash",
  "Backslash",
  "Semicolon",
  "Quote",
  "BracketLeft",
  "BracketRight",
  "Minus",
  "Equal",
  "Backquote",
]);

/** The non-modifier half of a shortcut, or null if this key cannot be one. */
const keyName = (code: string): string | null => {
  // `event.code` rather than `event.key`, and this is the reason the whole
  // function exists. `key` reports the character the layout *produces*, which
  // changes with the modifiers held: on macOS, Alt+A gives "å", and Shift+2
  // gives "@" on a US layout but a quotation mark on many others. Binding either
  // of those would store a shortcut that depends on the keyboard layout at the
  // moment it was recorded. `code` names the physical key, so Alt+A is always
  // "KeyA" and the binding survives a layout change.
  if (MODIFIER_CODES.has(code)) return null;

  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1];

  const digit = /^(?:Digit|Numpad)(\d)$/.exec(code);
  if (digit) return digit[1];

  if (/^F([1-9]|1\d|2[0-4])$/.test(code)) return code;

  if (RENAMED[code]) return RENAMED[code];
  if (VERBATIM.has(code)) return code;

  return null;
};

/**
 * Builds an accelerator from a keypress.
 *
 * Returns null while the press cannot yet be a shortcut — only modifiers held, an
 * unmappable key, or no modifier at all. A null means "keep listening" rather
 * than "reject": the natural way to type a combination is modifiers first, so
 * every capture passes through several null presses before the real one.
 */
export const acceleratorFrom = (event: KeyboardEvent): string | null => {
  const key = keyName(event.code);
  if (!key) return null;

  const parts: string[] = [];
  // Fixed order, so the same combination always yields the same string. Without
  // it, `Shift+Alt+M` and `Alt+Shift+M` would be different values for one
  // shortcut, and comparing the stored one against a new capture would be
  // unreliable.
  if (event.metaKey) parts.push("Command");
  if (event.ctrlKey) parts.push("Control");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");

  // A bare key is refused here as well as in Rust. Not defence in depth so much
  // as the difference between a good error and a confusing one: the user gets
  // "keep holding a modifier" from the control they are looking at, instead of a
  // rejection from the backend after the fact.
  if (parts.length === 0) return null;

  parts.push(key);
  return parts.join("+");
};

/** Whether this looks like a Mac, for display purposes only. */
const isMac = (): boolean =>
  typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.userAgent);

const MAC_SYMBOLS: Record<string, string> = {
  Command: "⌘",
  Cmd: "⌘",
  CmdOrCtrl: "⌘",
  CommandOrControl: "⌘",
  Control: "⌃",
  Ctrl: "⌃",
  Alt: "⌥",
  Option: "⌥",
  Shift: "⇧",
  Super: "⌘",
  Meta: "⌘",
};

/**
 * The shortcut as a person reads it: `⌥Space` rather than `Alt+Space`.
 *
 * Presentation only — the stored value stays in accelerator form. Rendering the
 * symbols matters because they are what every other macOS application shows and
 * what is printed on the keys, so "Alt" next to a key labelled ⌥ reads as a
 * different modifier to anyone who has not seen the Tauri syntax.
 */
export const describeShortcut = (accelerator: string): string => {
  if (!accelerator) return "";
  const parts = accelerator.split("+").map((p) => p.trim());
  if (!isMac()) return parts.join(" + ");

  return parts
    .map((part, index) => {
      const symbol = MAC_SYMBOLS[part];
      if (symbol) return symbol;
      // A space between the last modifier and a word-length key, so ⌥Space reads
      // as two things rather than one. Single characters need no gap: ⌘⇧M.
      return index > 0 && part.length > 1 ? ` ${part}` : part;
    })
    .join("");
};
