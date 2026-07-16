# catalyst/linux-gui — Linux test container

Ephemeral Ubuntu 24.04 container for the `test_env` tool's Linux backend.
Headless-capable (`podman exec`) and GUI-capable (Xvfb + x11vnc + noVNC +
websockify) so the agent can VNC into the screen for webui/GUI testing.

## Build

```bash
podman build -t catalyst/linux-gui:24.04 .
# (or docker build -t catalyst/linux-gui:24.04 .)
```

Point the tool at it: `CATALYST_TESTENV_LINUX_IMAGE=catalyst/linux-gui:24.04`

## What's inside

| component | purpose |
|---|---|
| Xvfb | virtual framebuffer on `:99` (1280x800x24) |
| openbox | lightweight WM so browser/GUI windows render |
| x11vnc | VNC server on `5900` mirroring the framebuffer |
| websockify + noVNC | web VNC client on `6080` (browser connects here) |
| Firefox | browser for webui tests |
| xdotool | mouse/keyboard input (the `input` action) |
| scrot | screenshots (the `screenshot` action) |
| curl/git/build-essential | common test tooling |

## Ports

- `5900` — raw VNC (for a native VNC viewer)
- `6080` — noVNC web client (open `http://<host>:6080/vnc.html`)

The `test_env` tool maps these to random host ports and hands the agent a
`ws://` URL for the noVNC websocket.
