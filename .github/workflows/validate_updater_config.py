"""Reject updater configurations that would panic during application startup."""

import json
import os
import re
import sys
from pathlib import Path
from urllib.parse import urlparse


root = Path(__file__).parents[2]
config_path = root / "src-tauri" / "tauri.conf.json"
config = json.loads(config_path.read_text())
updater = config["plugins"]["updater"]
endpoints = updater.get("endpoints", [])

app_version = config["version"]
cargo_version = re.search(
    r'^version\s*=\s*"([^"]+)"',
    (root / "src-tauri" / "Cargo.toml").read_text(),
    re.MULTILINE,
).group(1)
if app_version != cargo_version:
    sys.exit(
        f"version mismatch: tauri.conf.json={app_version}, "
        f"Cargo.toml={cargo_version}"
    )

release_tag = os.environ.get("GITHUB_REF_NAME", "")
if release_tag.startswith("v") and release_tag != f"v{app_version}":
    sys.exit(
        f"release tag mismatch: tag={release_tag}, config=v{app_version}"
    )

if not endpoints:
    sys.exit("updater must configure at least one endpoint")

insecure = [
    endpoint
    for endpoint in endpoints
    if urlparse(endpoint).scheme.lower() not in {"https"}
]

if insecure and not updater.get("dangerousInsecureTransportProtocol", False):
    sys.exit(
        "insecure updater endpoint requires "
        "dangerousInsecureTransportProtocol=true"
    )

if insecure and not updater.get("requireSignedVersion", False):
    sys.exit(
        "insecure updater endpoint requires requireSignedVersion=true"
    )

print(f"updater config OK: {len(endpoints)} endpoint(s)")
