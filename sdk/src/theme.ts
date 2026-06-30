// Minimal headless theme — the symbols pi-web imports
// (`Theme`, `initTheme`, `ThemeColor`) from `@earendil-works/pi-coding-agent`.
//
// The PI `Theme` drives TUI colour rendering. In a headless server there is no
// terminal, so every styling method returns the input text unchanged (no ANSI).
// pi-web's `getHeadlessTheme()` constructs `new Theme(fg, bg, "truecolor")` with
// full colour tables and hands the instance to extension UI contexts; it never
// calls the colour methods directly, so a no-op theme is sufficient and correct
// for the server path. `ThemeColor`/`ThemeBg` mirror PI's unions verbatim so the
// `Record<ThemeColor, string|number>` tables pi-web builds type-check unchanged.

export type ThemeColor =
  | "accent"
  | "border"
  | "borderAccent"
  | "borderMuted"
  | "success"
  | "error"
  | "warning"
  | "muted"
  | "dim"
  | "text"
  | "thinkingText"
  | "userMessageText"
  | "customMessageText"
  | "customMessageLabel"
  | "toolTitle"
  | "toolOutput"
  | "mdHeading"
  | "mdLink"
  | "mdLinkUrl"
  | "mdCode"
  | "mdCodeBlock"
  | "mdCodeBlockBorder"
  | "mdQuote"
  | "mdQuoteBorder"
  | "mdHr"
  | "mdListBullet"
  | "toolDiffAdded"
  | "toolDiffRemoved"
  | "toolDiffContext"
  | "syntaxComment"
  | "syntaxKeyword"
  | "syntaxFunction"
  | "syntaxVariable"
  | "syntaxString"
  | "syntaxNumber"
  | "syntaxType"
  | "syntaxOperator"
  | "syntaxPunctuation"
  | "thinkingOff"
  | "thinkingMinimal"
  | "thinkingLow"
  | "thinkingMedium"
  | "thinkingHigh"
  | "thinkingXhigh"
  | "bashMode";

export type ThemeBg =
  | "selectedBg"
  | "userMessageBg"
  | "customMessageBg"
  | "toolPendingBg"
  | "toolSuccessBg"
  | "toolErrorBg";

export type ColorTable = Record<ThemeColor, string | number>;
export type ColorBgTable = Record<ThemeBg, string | number>;
export type ColorMode = "none" | "ansi" | "ansi256" | "truecolor";

const ALL_FG: readonly ThemeColor[] = [
  "accent", "border", "borderAccent", "borderMuted", "success", "error", "warning",
  "muted", "dim", "text", "thinkingText", "userMessageText", "customMessageText",
  "customMessageLabel", "toolTitle", "toolOutput", "mdHeading", "mdLink", "mdLinkUrl",
  "mdCode", "mdCodeBlock", "mdCodeBlockBorder", "mdQuote", "mdQuoteBorder", "mdHr",
  "mdListBullet", "toolDiffAdded", "toolDiffRemoved", "toolDiffContext", "syntaxComment",
  "syntaxKeyword", "syntaxFunction", "syntaxVariable", "syntaxString", "syntaxNumber",
  "syntaxType", "syntaxOperator", "syntaxPunctuation", "thinkingOff", "thinkingMinimal",
  "thinkingLow", "thinkingMedium", "thinkingHigh", "thinkingXhigh", "bashMode",
];
const ALL_BG: readonly ThemeBg[] = [
  "selectedBg", "userMessageBg", "customMessageBg", "toolPendingBg", "toolSuccessBg", "toolErrorBg",
];

let initialized = false;

/** Initialise the global theme (no-op in headless mode). Mirrors `initTheme()`. */
export function initTheme(): void {
  initialized = true;
}

function completeTable<T extends string>(keys: readonly T[], fallback: string): Record<T, string> {
  const out = {} as Record<T, string>;
  for (const k of keys) out[k] = fallback;
  return out;
}

export class Theme {
  readonly fgColors: ColorTable;
  readonly bgColors: ColorBgTable;
  readonly mode: ColorMode;

  constructor(fgColors: ColorTable, bgColors: ColorBgTable, mode: ColorMode = "truecolor", _options?: unknown) {
    this.fgColors = fgColors;
    this.bgColors = bgColors;
    this.mode = mode;
  }

  /** Apply a foreground colour. Headless: returns the text unchanged. */
  fg(_color: ThemeColor, text: string): string {
    return text;
  }
  bg(_color: ThemeBg, text: string): string {
    return text;
  }
  style(_color: ThemeColor, text: string): string {
    return text;
  }
  /** Resolve a foreground colour to its raw value (hex/number). */
  color(color: ThemeColor): string | number {
    return this.fgColors[color];
  }
  bgColor(color: ThemeBg): string | number {
    return this.bgColors[color];
  }
  getFgAnsi(_color: ThemeColor): string {
    return "";
  }
  getBgAnsi(_color: ThemeBg): string {
    return "";
  }
  get isInitialized(): boolean {
    return initialized;
  }
}

/** A neutral headless theme (gray-on-black) used as the server fallback. */
export function getHeadlessTheme(): Theme {
  initTheme();
  const gray = "#9ca3af";
  const fg: ColorTable = completeTable(ALL_FG, gray);
  const bg: ColorBgTable = completeTable(ALL_BG, "#09090b");
  return new Theme(fg, bg, "truecolor");
}
