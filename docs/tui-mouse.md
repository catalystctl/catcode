# TUI mouse interactions

The terminal UI treats visible controls as mouse targets while preserving the
transcript's drag-to-select behavior. A stationary left-button press and
release activates a target; moving the pointer cancels the click so selecting
text never triggers a nearby control.

## Clickable surfaces

- **Header:** click the active model label to open the model picker.
- **Banners:** click core recovery (`retry`/`quit`), update, OAuth, approval,
  intercom, and queued-message affordances. OAuth clicks copy the URL (and open
  only safe `https` or loopback `http` URLs).
- **Activity and goal panels:** click to expand/collapse them. The position bar
  jumps to the newest transcript line.
- **Mention flyout:** click a file/directory row to insert it. Loading, empty,
  and hint rows are intentionally inert.
- **Composer:** click to focus and place the cursor near the clicked text;
  attached-image removal is also clickable.
- **Footer:** click the visible contextual actions (send, queue, abort, steer,
  approval decisions, newline, or commands).
- **Ask/sudo flyouts:** click question rows, select arrows, submit/skip,
  approve/decline, or the password field. These overlays own the whole screen;
  clicks and wheel events cannot reach the transcript underneath.
- **Transcript and modals:** click disclosure rows, drag to select/copy, and
  use the existing modal list hit-testing. Mouse wheel scrolls the active
  surface.

The keyboard remains a complete equivalent. Click actions are routed through
existing key/action handlers where possible, so custom keybindings and input
safety guards apply to both input methods.
