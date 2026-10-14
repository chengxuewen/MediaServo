#!/bin/bash
# install.sh — MediaServo Host bare-metal deployment script (D104 Phase 1 deployment mechanism)
# Purpose: install the host-legacy daemon on edge devices/vehicles (no Docker environment)
# Prereq: build the binary first (cargo build -p mediaservo-host --release); keep it in the same directory as host.conf/service files
# Note: from Phase 2 on, Docker Compose is the primary deployment path (D110); this script keeps edge-deploy capability
set -e

PREFIX="${PREFIX:-/opt/mediaservo}"
echo "Installing MediaServo Host to $PREFIX..."

BIN_NAME="host-legacy"

# Binary existence check (guards against silent failure — PIT-39 lesson)
if [ ! -f "${BIN_NAME}" ]; then
    echo "ERROR: ${BIN_NAME} not found in current directory." >&2
    echo "Build first: cargo build -p mediaservo-host --release" >&2
    exit 1
fi

# Create directory structure
mkdir -p "$PREFIX/bin" "$PREFIX/etc" "$PREFIX/web" "$PREFIX/logs"

# Copy binary
cp "${BIN_NAME}" "$PREFIX/bin/"
chmod +x "$PREFIX/bin/${BIN_NAME}"

# Copy default config (don't overwrite existing)
if [ -f host.conf ]; then
    if [ ! -f "$PREFIX/etc/host.conf" ]; then
        cp host.conf "$PREFIX/etc/host.conf"
        echo "Default config created: $PREFIX/etc/host.conf"
    else
        echo "Existing config preserved: $PREFIX/etc/host.conf"
    fi
else
    echo "Note: host.conf not found next to installer; skipping config copy"
fi

# Register systemd service
if [ -f "${BIN_NAME}.service" ] && command -v systemctl &> /dev/null; then
    cp "${BIN_NAME}.service" /etc/systemd/system/
    systemctl daemon-reload
    systemctl enable "${BIN_NAME}"
    echo "systemd service registered. Edit $PREFIX/etc/host.conf then:"
    echo "  systemctl start ${BIN_NAME}"
else
    echo "No systemd unit found (or systemd unavailable). Start manually:"
    echo "  $PREFIX/bin/${BIN_NAME} --config $PREFIX/etc/host.conf"
fi

echo "Installation complete."
