# Desktop image for iii-sandbox-backed computer-use sessions.
#
# iii-sandbox boots this image's filesystem as a libkrun microVM rootfs. It does
# NOT run the ENTRYPOINT/CMD (PID 1 is the engine's supervisor), so nothing here
# autostarts: the computer worker brings up Xvfb + openbox via `sandbox::exec`
# on session start. This image only needs the tools present on the filesystem.
#
# The worker drives the guest with:
#   - Xvfb           the virtual display (fixed resolution, e.g. 1280x800)
#   - openbox        a minimal window manager
#   - xdotool        pointer + keyboard injection (DISPLAY=:0)
#   - imagemagick    `import` grabs the X root to JPEG for screenshots
#   - x11-utils      `xdpyinfo` for the display-readiness probe
#   - procps         `pgrep` used by the bootstrap
#   - util-linux     `setsid` to detach Xvfb into its own session
#
# Build and publish, then register it with the sandbox worker as a custom image:
#
#   docker build -t ghcr.io/<you>/iii-desktop:latest computer/images/desktop
#   docker push  ghcr.io/<you>/iii-desktop:latest
#
#   # engine config.yaml, under the iii-sandbox worker:
#   #   custom_images:
#   #     desktop: ghcr.io/<you>/iii-desktop:latest
#   #   image_allowlist:
#   #     - desktop
#
# Then: computer::sessions::start { "image": "desktop" }

FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive \
    DISPLAY=:0

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      xvfb \
      x11-utils \
      xdotool \
      imagemagick \
      openbox \
      procps \
      util-linux \
      dbus-x11 \
      fonts-dejavu-core \
      ca-certificates \
      xterm \
 && rm -rf /var/lib/apt/lists/*

# ImageMagick's default policy blocks some coders; the X-root grab used for
# screenshots is unaffected, but relax the width/height/memory limits so a
# full-screen capture is never rejected on resource policy.
RUN if [ -f /etc/ImageMagick-6/policy.xml ]; then \
      sed -i 's/name="memory" value="[^"]*"/name="memory" value="512MiB"/' /etc/ImageMagick-6/policy.xml; \
      sed -i 's/name="width" value="[^"]*"/name="width" value="32KP"/'     /etc/ImageMagick-6/policy.xml; \
      sed -i 's/name="height" value="[^"]*"/name="height" value="32KP"/'   /etc/ImageMagick-6/policy.xml; \
    fi
