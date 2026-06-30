// Extension runtime — mirrors `pi-coding-agent`'s `core/extensions/` (the subset
// pi-web touches). The `ExtensionUIContext` interface matches the one pi-web's
// `createExtensionUIContext()` implements; `ExtensionRunner` exposes a mutable
// `emit` (pi-web monkey-patches it to intercept `model_select` /
// `thinking_level_select`) and `getRegisteredCommands()`.

import type { Theme, ThemeColor } from "./theme.js";

export type WidgetPlacement = "aboveEditor" | "belowEditor";

export interface ExtensionUIDialogOptions {
  timeout?: number;
}

export interface ExtensionWidgetOptions {
  placement?: WidgetPlacement;
}

export interface ExtensionError {
  extensionPath?: string;
  event?: string;
  error?: string;
}

export type ExtensionErrorListener = (err: ExtensionError) => void;

export interface RegisteredCommand {
  invocationName: string;
  description?: string;
  source?: string;
  sourceInfo?: { location?: string; path?: string };
}

export interface ExtensionBindings {
  uiContext?: ExtensionUIContext;
  commandContextActions?: any;
  abortHandler?: () => void;
  shutdownHandler?: () => void;
  onError?: ExtensionErrorListener;
}

export interface ExtensionUIContext {
  select(title: string, options: string[], opts?: ExtensionUIDialogOptions): Promise<string | undefined>;
  confirm(title: string, message: string, opts?: ExtensionUIDialogOptions): Promise<boolean>;
  input(title: string, placeholder: string, opts?: ExtensionUIDialogOptions): Promise<string | undefined>;
  notify(message: string, type?: "info" | "success" | "warning" | "error"): void;
  onTerminalInput(): () => void;
  setStatus(key: string, text: string): void;
  setWorkingMessage(message?: string): void;
  setWorkingVisible(visible: boolean): void;
  setWorkingIndicator(indicator?: string): void;
  setHiddenThinkingLabel(label?: string): void;
  setWidget(key: string, content?: string[], options?: ExtensionWidgetOptions): void;
  setFooter(text?: string): void;
  setHeader(text?: string): void;
  setTitle(title: string): void;
  custom<T = unknown>(factory: any, options?: any): Promise<T>;
  pasteToEditor(text: string): void;
  setEditorText(text: string): void;
  getEditorText(): string;
  editor(title: string, prefill?: string): Promise<string | undefined>;
  addAutocompleteProvider(provider: any): void;
  setEditorComponent(component: any): void;
  getEditorComponent(): any;
  readonly theme: Theme;
  getAllThemes(): Theme[];
  getTheme(): Theme;
  setTheme(name: string): { success: boolean; error?: string };
  getToolsExpanded(): boolean;
  setToolsExpanded(expanded: boolean): void;
}

export type ExtensionEvent =
  | { type: "model_select"; model: { provider: string; id: string; name?: string } }
  | { type: "thinking_level_select"; level: string }
  | { type: string; [key: string]: unknown };

/** Minimal extension runner. `emit` is an instance property (not a prototype
 *  method) so consumers can replace it — exactly what pi-web does to intercept
 *  `model_select` / `thinking_level_select`. */
export class ExtensionRunner {
  private commands: RegisteredCommand[] = [];
  private emitListeners: Array<(event: ExtensionEvent) => Promise<void> | void> = [];

  /** Mutable emit. Calling it forwards to registered listeners. Reassign to
   *  intercept (pi-web wraps it). */
  emit: (event: ExtensionEvent) => Promise<void> = async (event) => {
    for (const fn of this.emitListeners) {
      try {
        await fn(event);
      } catch {
        /* listener errors are non-fatal */
      }
    }
  };

  /** Register an internal listener (used by AgentSession to forward events). */
  onEmit(fn: (event: ExtensionEvent) => Promise<void> | void): () => void {
    this.emitListeners.push(fn);
    return () => {
      this.emitListeners = this.emitListeners.filter((l) => l !== fn);
    };
  }

  registerCommand(command: RegisteredCommand): void {
    this.commands.push(command);
  }

  getRegisteredCommands(): RegisteredCommand[] {
    return [...this.commands];
  }
}

export { Theme, ThemeColor };
