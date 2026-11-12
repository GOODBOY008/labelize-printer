#!/usr/bin/env python3
"""Read the shared version for release packages."""

from pathlib import Path
import re
import tomllib

manifest = Path(__file__).resolve().parents[2] / "Cargo.toml"
with manifest.open("rb") as source:
    version = tomllib.load(source)["workspace"]["package"]["version"]

if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
    raise SystemExit("Installers require a numeric X.Y.Z workspace version")
print(version)
