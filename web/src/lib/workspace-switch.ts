export interface DirtyWorkspaceTab {
  label: string;
  dirty?: boolean;
}

export function dirtyWorkspaceSwitchMessage(tabs: DirtyWorkspaceTab[]): string | null {
  const dirty = tabs.filter((tab) => tab.dirty);
  if (dirty.length === 0) return null;
  const names = dirty.slice(0, 3).map((tab) => tab.label).join(", ");
  const remainder = dirty.length > 3 ? ` and ${dirty.length - 3} more` : "";
  return `Discard unsaved changes in ${names}${remainder} and switch projects?`;
}

/**
 * Central decision used by every workspace-switch UI entry point.
 * Returning false means the caller must leave the current workspace intact.
 */
export function allowWorkspaceSwitch(
  currentWorkspace: string,
  targetWorkspace: string,
  tabs: DirtyWorkspaceTab[],
  confirmDiscard: (message: string) => boolean,
): boolean {
  if (!targetWorkspace || targetWorkspace === currentWorkspace) return false;
  const message = dirtyWorkspaceSwitchMessage(tabs);
  return message ? confirmDiscard(message) : true;
}
