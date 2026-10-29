"""Generate Tauri updater manifest (update.json) from release signature files.

URL scheme: downloads go through the China-friendly VPS proxy
(sub.hugme.space/landrop/<file>) which reverse-proxies GitHub Releases.
"""
import json
import sys
import glob
import os
from datetime import datetime, timezone

tag = sys.argv[1]                    # e.g. v0.1.1
version = tag.lstrip("v")
BASE = "https://sub.hugme.space/landrop"


def read_sig(pattern):
    files = sorted(glob.glob(pattern))
    if not files:
        return "", ""
    sig = open(files[0]).read().strip()
    asset = os.path.basename(files[0])[: -len(".sig")]  # strip .sig
    return sig, asset


platforms = {}

mac_sig, mac_asset = read_sig("sigs/*.app.tar.gz.sig")
if mac_sig:
    platforms["darwin-aarch64"] = {
        "signature": mac_sig,
        "url": f"{BASE}/{mac_asset}",
    }

# Windows: prefer NSIS updater artifact, fall back to MSI
win_sig, win_asset = read_sig("sigs/*x64-setup.exe.sig")
if not win_sig:
    win_sig, win_asset = read_sig("sigs/*.msi.sig")
if win_sig:
    platforms["windows-x86_64"] = {
        "signature": win_sig,
        "url": f"{BASE}/{win_asset}",
    }

if not platforms:
    sys.exit("no signature files found in sigs/")

data = {
    "version": version,
    "notes": f"LanDrop {tag}",
    "pub_date": datetime.now(timezone.utc).isoformat(),
    "platforms": platforms,
}

with open("update.json", "w") as f:
    json.dump(data, f, indent=2, ensure_ascii=False)

print(json.dumps(data, indent=2, ensure_ascii=False)[:500])
print("update.json generated OK")
