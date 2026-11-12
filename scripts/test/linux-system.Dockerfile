FROM labelize-printer-build
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    cups cups-client cups-filters policykit-1 xdg-desktop-portal xdg-desktop-portal-gtk \
    dbus-daemon mesa-vulkan-drivers openbox weston x11-utils xvfb \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
