# Catalyst Code Frontend Direction

## Subject

Catalyst Code is a live coding-agent workbench for developers. Its audience is a developer supervising long-running model work across projects and sessions. The frontend has one job: keep human prompts, agent output, tool activity, decisions, and repository state legible as one continuous working record.

## Token System

The interface uses the existing Obsidian and ember identity. Ember means human intent or active work; neutral ink layers carry machine output and chrome; semantic colors are reserved for state.

| Token | Dark | Light | Job |
| --- | --- | --- | --- |
| Ground | `#0e0f12` | `#f8f8fa` | Uninterrupted workspace canvas |
| Recessed | `#111317` | `#f3f3f6` | Code, tool output, and input wells |
| Surface | `#16181d` | `#ffffff` | Panels and durable controls |
| Ink | `#f4f5f8` | `#16161b` | Primary text and icons |
| Ember | `#d68e58` | `#b66e3a` | Human intent, focus, and live work |
| Signal cyan | `#38c7c9` | `#0c7490` | Informational state only |

Semantic success, warning, and danger colors do not become decorative accents.

### Type

- Display: Outfit Variable, 600, used only for the product mark and true empty-state headline.
- Body: DM Sans Variable, 400/500/600, for readable conversational and control text.
- Utility: JetBrains Mono Variable, 400/500, for models, paths, metrics, role labels, tools, and protocol state.
- Scale: 10px utility, 11px metadata, 12px compact control, 13.5px transcript body, 16px panel title, 24px empty-state display. Letter spacing is `0` for prose and headings; only short uppercase protocol labels use `0.12em` to `0.14em`.

### Layout

Concept: a project switchboard wraps a continuous session ledger, with repository state docked beside it and the prompt instrument fixed at hand.

Selected:

```text
+ project tabs / project controls ---------------------------+
| sessions |  YOU o                                         | git |
|          |      | prompt                                  |     |
|          | AGENT o response + tools                       |     |
|          |       |                                        |     |
|          |  META . tool result                            |     |
|          + fixed prompt instrument -----------------------+     |
+------------------------------------------------------------+
```

Rejected alternative A: chat bubbles between a navigation rail and a grid of status cards. It fragments the working record and reads like a generic messenger dashboard.

Rejected alternative B: terminal-first split panes. It makes raw process chrome the product and weakens the primary supervision workflow.

## Signature

A continuous turn rail runs through every session. Human prompts use ember ticks, agent turns use neutral ticks, tools use smaller protocol ticks, and the active agent tick pulses. It makes chronology, authorship, and live state scannable in the visual language of traces and execution logs.

This is the one expressive move. Surrounding panels stay quiet, compact, and mostly rectangular; gradients appear only as shallow depth or directional emphasis, never as spectacle.

## Brief Review

Initial ideas included a dotted technical backdrop, glowing live states, a large empty-state card, and several raised control wells. Together they pushed the product toward a familiar AI developer-tool skin. The revised direction keeps the trace-derived rail, removes ambient glow as a general motif, limits elevation to transient overlays and the prompt instrument, and uses the dotted field only where it helps distinguish an empty workspace from an active record.

## Current Default Cluster

In addition to the mid-2025 defaults in the skill, current generators overproduce: dark developer dashboards with violet or cyan glow; oversized rounded command bars; dense icon rails with detached floating panels; animated dot-grid backgrounds; pill badges for every state; fake terminal snippets as identity; and glassy command palettes. These are treated as defaults to replace unless they encode real Catalyst Code behavior.

## Reverse-5

A templated generator would likely:

1. Use dark navy, violet glow, and a gradient product name.
2. Put user and assistant text in alternating rounded chat bubbles.
3. Open with a centered welcome headline and four icon suggestion cards.
4. Turn metrics, tools, git, and sessions into a bento card dashboard.
5. Animate a dot grid, typing cursor, status pills, and hover lifts simultaneously.

The plan matches none of the first four. It retains one restrained cursor/live-state motion because streaming is real system state, and the turn-rail pulse is the signature itself.

## Accessibility And Motion

Both explicit light and dark themes must meet WCAG AA. Every interactive element gets a visible `:focus-visible` state and a minimum 44px target on touch layouts. The shell must fit at 320px without horizontal page scroll. Dialogs and drawers trap focus and preserve semantic landmarks. Reduced-motion mode disables entrance, rail pulse, shimmer, lift, and sheet transitions while preserving state through color and copy.

## Critique Checklist

- Does the turn rail read immediately on populated sessions?
- Is ember reserved for intent, focus, and live state?
- Does any panel look like a decorative card or card-inside-card?
- Are tool output, repository state, and prompts readable without competing for attention?
- Do controls remain reachable and text remain untruncated at 320px?
- Does light mode retain depth and contrast without becoming a different brand?
