#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
binary_dir="${1:-target/release}"
runtime_dir="$(mktemp -d)"
xvfb_pid=
wm_pid=
weston_pid=
cleanup() {
    local result=$?
    if [[ -n "$weston_pid" ]]; then kill "$weston_pid" 2>/dev/null || true; fi
    if [[ -n "$wm_pid" ]]; then kill "$wm_pid" 2>/dev/null || true; fi
    if [[ -n "$xvfb_pid" ]]; then kill "$xvfb_pid" 2>/dev/null || true; fi
    wait 2>/dev/null || true
    if (( result != 0 )); then
        tail -n 40 "$runtime_dir/"*.log >&2 || true
    fi
    rm -rf "$runtime_dir"
}
trap cleanup EXIT
export XDG_RUNTIME_DIR="$runtime_dir"
export LIBGL_ALWAYS_SOFTWARE=1

Xvfb -displayfd 1 -screen 0 1280x900x24 -nolisten tcp \
    > "$runtime_dir/display" 2> "$runtime_dir/xvfb.log" &
xvfb_pid=$!
for ((attempt = 0; attempt < 100; attempt++)); do
    if [[ -s "$runtime_dir/display" ]]; then break; fi
    kill -0 "$xvfb_pid"
    sleep 0.1
done
[[ "$attempt" -lt 100 ]] || { echo 'Xvfb did not become ready' >&2; exit 1; }
export DISPLAY=":$(< "$runtime_dir/display")"

openbox --sm-disable > "$runtime_dir/openbox.log" 2>&1 &
wm_pid=$!
for ((attempt = 0; attempt < 100; attempt++)); do
    if [[ "$(xprop -root _NET_SUPPORTING_WM_CHECK)" == *'window id #'* ]]; then break; fi
    kill -0 "$wm_pid"
    sleep 0.1
done
[[ "$attempt" -lt 100 ]] || { echo 'Openbox did not become ready' >&2; exit 1; }
echo 'Checking Linux X11'
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \
    dbus-run-session -- python3 scripts/test/desktop-smoke.py --bin-dir "$binary_dir"

# GPUI expects an input seat. The nested X11 backend supplies virtual input.
weston --backend=x11-backend.so --use-pixman --shell=kiosk-shell.so \
    --width=1280 --height=900 --socket=studio-smoke --idle-time=0 > "$runtime_dir/weston.log" 2>&1 &
weston_pid=$!
for ((attempt = 0; attempt < 100; attempt++)); do
    if [[ -S "$runtime_dir/studio-smoke" ]]; then break; fi
    kill -0 "$weston_pid"
    sleep 0.1
done
[[ "$attempt" -lt 100 ]] || { echo 'Weston did not become ready' >&2; exit 1; }
echo 'Checking Linux Wayland'
env -u DISPLAY -u WAYLAND_SOCKET WAYLAND_DISPLAY=studio-smoke \
    dbus-run-session -- python3 scripts/test/desktop-smoke.py --bin-dir "$binary_dir"
