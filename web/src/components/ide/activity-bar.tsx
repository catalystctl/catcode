"use client";

// Activity bar — the far-left 48px vertical icon strip (VSCode analog). It
// keeps Explorer fixed and toggles/focuses each movable dock panel.
//
// Consumes { ide } from IdeContext. The copilot (Spark) entry at the bottom
// toggles copilotVisible — the Chat dock itself is hosted by IdeShell, not here.

import { useIdeContext } from "@/lib/ide-context";
import { SparkIcon } from "@/components/icons";
import { PANELS, PANEL_ORDER } from "./panel-registry";
import type { IdePanelId } from "@/lib/types";

export function ActivityBar() {
  const { ide } = useIdeContext();
  const {
    sidebarCollapsed,
    terminals,
  } = ide.state;

  const requestedLeft = ide.state.activeDockPanels.left;
  const leftPanelActive =
    requestedLeft !== null &&
    ide.state.panelVisibility[requestedLeft] &&
    ide.state.panelLocations[requestedLeft] === "left";

  const onPanelClick = (p: IdePanelId) => {
    if (p === "explorer") {
      if (!sidebarCollapsed && !leftPanelActive) ide.toggleSidebar();
      else ide.selectExplorer();
      return;
    }
    if (p === "terminal") {
      if (terminals.length === 0) {
        ide.newTerminal();
        ide.showDockPanel("terminal");
        return;
      }
    }
    ide.toggleDockPanel(p);
  };

  const isActive = (p: IdePanelId): boolean =>
    p === "explorer" ? !sidebarCollapsed && !leftPanelActive : ide.state.panelVisibility[p];

  return (
    <div className="flex w-12 shrink-0 flex-col items-center gap-1 border-r border-ink-800 bg-ink-925 py-2">
      {PANEL_ORDER.map((p) => {
        const desc = PANELS[p];
        const Icon = desc.icon;
        const active = isActive(p);
        return (
          <button
            key={p}
            type="button"
            title={desc.label}
            aria-label={desc.label}
            aria-pressed={active}
            onClick={() => onPanelClick(p)}
            className={`relative flex h-10 w-10 items-center justify-center rounded-md transition-colors ${
              active ? "text-ink-100" : "text-ink-500 hover:bg-ink-800/60 hover:text-ink-200"
            }`}
          >
            <Icon width={22} height={22} />
            {active ? (
              <span className="absolute left-0 top-1/2 h-6 w-0.5 -translate-y-1/2 rounded-r bg-accent" />
            ) : null}
          </button>
        );
      })}

      <div className="flex-1" />

      <button
        type="button"
        title="Copilot (Chat)"
        aria-label="Copilot (Chat)"
        aria-pressed={ide.state.panelVisibility.chat}
        onClick={() => ide.toggleCopilot()}
        className={`relative flex h-10 w-10 items-center justify-center rounded-md transition-colors ${
          ide.state.panelVisibility.chat ? "text-ink-100" : "text-ink-500 hover:bg-ink-800/60 hover:text-ink-200"
        }`}
      >
        <SparkIcon width={22} height={22} />
        {ide.state.panelVisibility.chat ? (
          <span className="absolute left-0 top-1/2 h-6 w-0.5 -translate-y-1/2 rounded-r bg-accent" />
        ) : null}
      </button>
    </div>
  );
}
