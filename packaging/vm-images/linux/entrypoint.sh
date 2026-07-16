#!/usr/bin/env bash
# entrypoint.sh — start the GUI stack for the catalyst/linux-gui container.
#   Xvfb (virtual framebuffer :99) → openbox (WM) → x11vnc (VNC on 5900)
#   → websockify (noVNC web client on 6080, bridging to 5900) → wait.
set -euo pipefail

export DISPLAY="${DISPLAY:-:99}"
RES="${RESOLUTION:-1280x800x24}"

# 1. Virtual framebuffer.
Xvfb "$DISPLAY" -screen 0 "$RES" -ac +extension RANDR &
XVFB_PID=$!

# Give Xvfb a moment to come up.
sleep 1

# 2. Lightweight window manager (so GUI apps / browser windows render properly).
openbox &

# 3. VNC server mirroring the framebuffer. No password (ephemeral, isolated).
x11vnc -display "$DISPLAY" -forever -shared -noxdamage -rfbport "${VNC_PORT:-5900}" \
    -bg -o /tmp/x11vnc.log -quiet

# 4. websockify: serve the noVNC web client on 6080, bridging to the VNC port.
websockify --web=/usr/share/novnc "${NOVNC_PORT:-6080}" "localhost:${VNC_PORT:-5900}" \
    > /tmp/websockify.log 2>&1 &

echo "[entrypoint] GUI stack up: VNC on ${VNC_PORT:-5900}, noVNC on http://localhost:${NOVNC_PORT:-6080}/vnc.html"
echo "[entrypoint] DISPLAY=$DISPLAY RES=$RES"

# 5. Keep the container alive; reap Xvfb if it dies.
wait "$XVFB_PID"
