#!/usr/bin/env python3
"""Refresh dependency notices from the installed, locked Rust and browser dependencies."""
import json
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parent.parent
metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1"], cwd=root))
out = ["# Third-party dependency notices\n\nGenerated from Cargo.lock and web/package-lock.json. Build/dev Rust dependencies are included as well.\n"]
for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
    if package["name"] == "rustysoul":
        continue
    directory = Path(package["manifest_path"]).parent
    out.append(f'\n## {package["name"]} {package["version"]}\n\nLicense declaration: {package.get("license") or "see package"}\n')
    for file in sorted(directory.iterdir()):
        if file.is_file() and file.name.upper().startswith(("LICENSE", "COPYING", "NOTICE", "UNLICENSE")):
            out.append(f'\n### {file.name}\n\n```text\n{file.read_text(errors="replace")}\n```\n')
lock = json.loads((root / "web/package-lock.json").read_text())
for name, package in sorted(lock["packages"].items()):
    if not name or package.get("dev"):
        continue
    directory = root / "web" / name
    if not directory.is_dir():
        continue
    out.append(f'\n## npm: {name.removeprefix("node_modules/")} {package["version"]}\n')
    for file in sorted(directory.iterdir()):
        if file.is_file() and file.name.upper().startswith(("LICENSE", "COPYING", "NOTICE")):
            out.append(f'\n### {file.name}\n\n```text\n{file.read_text(errors="replace")}\n```\n')
(root / "THIRD_PARTY_NOTICES.md").write_text("\n".join(out))
