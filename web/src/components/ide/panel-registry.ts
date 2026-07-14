"use client";

// Panel registry — the single import surface for the IDE shell. It maps each
// IdePanelId to its activity-bar descriptor (label + icon) and centralizes the
// panel component imports (the prior-step panels at their exact contract paths:
// ./file-tree, ./editor, ./terminal, ./git-panel, ./preview).
//
// Heavy panels (editor, terminal, preview) are loaded via next/dynamic with
// ssr:false so CodeMirror / Ghostty WASM / iframe logic never run on the server and
// never enter the main bundle chunk — only the active panel's chunk loads.
// Light panels (file-tree, git-panel) are static imports (they consume the
// IdeContext and carry no heavy deps).

import dynamic from "next/dynamic";
import type { ComponentType } from "react";
import {
  FolderIcon,
  GitBranchIcon,
  TerminalIcon,
  GlobeIcon,
} from "@/components/icons";
import type { IdePanelId } from "@/lib/types";

// Light panels — static import (use useIdeContext for { workspace, ide }).
import { FileTree } from "./file-tree";
import { GitPanel } from "./git-panel";

// Heavy panels — dynamic, client-only.
const Editor = dynamic(() => import("./editor").then((m) => m.Editor), { ssr: false });
const Terminal = dynamic(() => import("./terminal").then((m) => m.Terminal), { ssr: false });
const TerminalPanel = dynamic(() => import("./terminal").then((m) => m.TerminalPanel), {
  ssr: false,
});
const Preview = dynamic(() => import("./preview").then((m) => m.Preview), { ssr: false });

export type IconProps = { width?: number; height?: number; className?: string };

export interface PanelDescriptor {
  id: IdePanelId;
  label: string;
  icon: ComponentType<IconProps>;
}

/** Activity-bar descriptors for the four switchable panels. */
export const PANELS: Record<IdePanelId, PanelDescriptor> = {
  explorer: { id: "explorer", label: "Explorer", icon: FolderIcon },
  git: { id: "git", label: "Source Control", icon: GitBranchIcon },
  terminal: { id: "terminal", label: "Terminal", icon: TerminalIcon },
  preview: { id: "preview", label: "Preview", icon: GlobeIcon },
};

/** Order the activity bar renders the panel icons (copilot is appended after). */
export const PANEL_ORDER: IdePanelId[] = ["explorer", "git", "terminal", "preview"];

// Re-export the panel components so the shell imports everything from here.
export { FileTree, GitPanel, Editor, Terminal, TerminalPanel, Preview };
