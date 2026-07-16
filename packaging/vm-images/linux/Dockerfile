# catalyst/linux-gui:24.04 — ephemeral Linux test container for the test_env tool.
#
# Headless-capable (podman exec) AND GUI-capable (Xvfb + x11vnc + noVNC +
# websockify) so the agent can VNC into the screen for webui/GUI testing.
# Firefox is included for webui tests; xdotool + scrot for input + screenshots.
FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive \
    DISPLAY=:99 \
    RESOLUTION=1280x800x24 \
    VNC_PORT=5900 \
    NOVNC_PORT=6080

RUN apt-get update && apt-get install -y --no-install-recommends \
        xvfb x11vnc openbox xdotool scrot \
        novnc websockify \
        firefox \
        fonts-liberation fonts-dejavu-core \
        openssh-client ca-certificates curl wget git \
        bash build-essential \
    && rm -rf /var/lib/apt/lists/*

# noVNC ships its web client under /usr/share/novnc; websockify serves it.
# A password-less VNC is fine for an isolated, ephemeral test container.
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

EXPOSE 5900 6080
ENTRYPOINT ["/entrypoint.sh"]
