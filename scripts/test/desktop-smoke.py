#!/usr/bin/env python3
"""Launch GPUI in the active desktop session and verify a two-page preview."""

import argparse
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
EXPECTED = [(102.0, 152.0, 816, 1216), (75.0, 50.0, 600, 400)]


def check_preview(manifest):
    pages = json.loads(manifest.read_text(encoding="utf-8"))
    sizes = [(p["width_mm"], p["height_mm"], p["width"], p["height"]) for p in pages]
    if sizes != EXPECTED:
        raise AssertionError(f"Unexpected preview sizes: {sizes}")
    for page, (_, _, width, height) in zip(pages, EXPECTED):
        with (manifest.parent / page["file"]).open("rb") as image:
            header = image.read(24)
        if len(header) != 24 or header[:8] != b"\x89PNG\r\n\x1a\n":
            raise AssertionError(f"Invalid preview PNG: {page['file']}")
        if struct.unpack(">II", header[16:24]) != (width, height):
            raise AssertionError(f"Unexpected PNG dimensions: {page['file']}")


def stop_daemon(cli):
    stopped = subprocess.run([*cli, "stop"], capture_output=True, timeout=5)
    if stopped.returncode != 0:
        return
    # The service exits shortly after acknowledging shutdown. Wait for it to
    # close its files before removing the state directory, especially on Windows.
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if subprocess.run([*cli, "status"], capture_output=True, timeout=5).returncode != 0:
            return
        time.sleep(0.1)
    raise TimeoutError("Background service did not stop")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/debug")
    args = parser.parse_args()
    suffix = ".exe" if os.name == "nt" else ""
    desktop = args.bin_dir.resolve() / f"labelize-printer{suffix}"
    daemon = args.bin_dir.resolve() / f"labelize-printer-daemon{suffix}"
    if not desktop.is_file() or not daemon.is_file():
        parser.error(f"Desktop and daemon binaries are required in {args.bin_dir}")

    with tempfile.TemporaryDirectory(prefix="studio-ui-") as directory:
        work = Path(directory).resolve()
        state = work / "state"
        log_path = work / "desktop.log"
        cli = [str(daemon), "--data-dir", str(state)]
        env = dict(os.environ, LABELIZE_PRINTER_HOME=str(state))
        with log_path.open("w", encoding="utf-8") as log:
            app = subprocess.Popen(
                [str(desktop), str(ROOT / "examples/shipping-demo.zpl")],
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
            )
            try:
                deadline = time.monotonic() + 45
                while time.monotonic() < deadline:
                    if app.poll() is not None:
                        raise RuntimeError(f"Desktop exited with status {app.returncode}")
                    manifest = next(state.glob("previews/document-*/pages.json"), None)
                    if manifest is not None:
                        check_preview(manifest)
                        status = subprocess.run(
                            [*cli, "status"], capture_output=True, timeout=5
                        )
                        if status.returncode == 0 and json.loads(status.stdout)["raw_listening"]:
                            print("PASS GPUI opened the ZPL example and rendered both preview pages")
                            return
                    time.sleep(0.2)
                raise TimeoutError("Desktop preview or background service did not become ready")
            except Exception:
                print(log_path.read_text(encoding="utf-8", errors="replace")[-6000:])
                raise
            finally:
                if app.poll() is None:
                    app.terminate()
                    try:
                        app.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        app.kill()
                        app.wait()
                stop_daemon(cli)


if __name__ == "__main__":
    main()
