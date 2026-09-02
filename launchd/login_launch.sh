#!/bin/bash
# Prompts once (native macOS password dialog) for administrator privileges,
# then runs the KVM-switch script as root under that elevation.
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$REPO_DIR/target/release/msi-monitor-ctrl"
SCRIPT="$REPO_DIR/kvm_switch.lua"

osascript -e "do shell script \"'$BIN' --cmd '$SCRIPT'\" with administrator privileges"
