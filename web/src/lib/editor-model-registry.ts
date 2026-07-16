// Lightweight bridge between IDE tab lifecycle and Monaco's long-lived models.
// This module deliberately has no Monaco import, so use-ide stays in the small
// initial client bundle and the editor remains lazy-loaded.

const disposers = new Map<string, () => void>();

export function registerEditorModel(tabId: string, dispose: () => void): void {
  disposers.set(tabId, dispose);
}

export function disposeEditorModel(tabId: string): void {
  disposers.get(tabId)?.();
  disposers.delete(tabId);
}

export function disposeAllEditorModels(): void {
  for (const dispose of disposers.values()) {
    dispose();
  }
  disposers.clear();
}
