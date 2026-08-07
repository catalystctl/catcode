export const SESSION_PREFERENCES_KEY = "catalyst.chat-session-preferences.v1";

export interface SessionPreferences {
  query: string;
  pinned: string[];
  archived: string[];
}

export const EMPTY_SESSION_PREFERENCES: SessionPreferences = {
  query: "",
  pinned: [],
  archived: [],
};

export function readSessionPreferences(scope?: string): SessionPreferences {
  if (typeof window === "undefined") return EMPTY_SESSION_PREFERENCES;
  try {
    const parsed = JSON.parse(window.localStorage.getItem(storageKey(scope)) ?? "null") as Partial<SessionPreferences> | null;
    return {
      query: typeof parsed?.query === "string" ? parsed.query : "",
      pinned: stringArray(parsed?.pinned),
      archived: stringArray(parsed?.archived),
    };
  } catch {
    return EMPTY_SESSION_PREFERENCES;
  }
}

export function writeSessionPreferences(preferences: SessionPreferences, scope?: string): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(storageKey(scope), JSON.stringify(preferences));
  } catch {
    // Browsers may disable storage. Session management remains fully usable.
  }
}

function storageKey(scope?: string): string {
  return scope ? `${SESSION_PREFERENCES_KEY}:${encodeURIComponent(scope)}` : SESSION_PREFERENCES_KEY;
}

function stringArray(value: unknown): string[] {
  return Array.isArray(value)
    ? [...new Set(value.filter((entry): entry is string => typeof entry === "string"))]
    : [];
}
