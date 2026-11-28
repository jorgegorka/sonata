#!/usr/bin/env python3
"""Scan only publication candidates, including staged content on an unborn branch."""
import argparse
import hashlib
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--staged", action="store_true")
args = parser.parse_args()
root = Path(subprocess.check_output(["git", "rev-parse", "--show-toplevel"]).decode().strip())
os.chdir(root)
names = subprocess.check_output(["git", "ls-files", "-z"] if args.staged else
    ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"]).decode().split("\0")
denied = re.compile(r"(?i)(^|/)(?:\.env(?:\..*)?|id_(?:rsa|ed25519|ecdsa).*|credentials(?:\..*)?|secrets?(?:\..*)?|\.npmrc|\.netrc|\.pypirc|\.mcp\.json|.*\.(?:pem|key|p8|p12|pfx|mobileprovision|provisionprofile|db(?:-[^/]*)?|sqlite3?(?:-[^/]*)?|log|har|dmg))$")
failures = []
# User-approved public installers are pinned to the notarized artifacts' digests.
public_artifacts = {
    "site/downloads/Sonata_1.0.12.dmg": "87de3d8a7947f1140a32e87803c4a1df6d22f1424902343f83a148d14371ceb4",
    "site/downloads/Sonata_1.0.12.app.tar.gz": "fef3dce1f05c844d5f33a937df97231e50a961a33fc517052670f115ae9ed9a7",

    "site/downloads/Sonata_1.0.11.dmg": "d3464053b2a912bb2e17c4f28c71e0ce1be3ab0dfa680a66687912bedc2fc066",
    "site/downloads/Sonata_1.0.11.app.tar.gz": "9d38751a10fc708df6ff2a8b34ebe4a8603b5383d4b3d97cbaa6fec9a4ab7f18",
    "site/downloads/Sonata_1.0.10.dmg": "6272eae16af921fcd5a9521fac2b621efe6c6ca7cf514c10e9988ea2d7217d5e",
    "site/downloads/Sonata_1.0.10.app.tar.gz": "ec0beb25c109ddd5ffdf87c163d66e676d6803b664e210f5fddec56f32c08e52",

    "site/downloads/Sonata_1.0.9.dmg": "ab5043bc58110648c49aefa2df843e1be597d5f87d74042ab1c290f7bbabdd1b",
    "site/downloads/Sonata_1.0.9.app.tar.gz": "82a32e4d3fdd96531b68eeb38b1c96fb69a8ffe0760765160c7f6d254511efb4",
    "site/downloads/Sonata_1.0.8.dmg": "79316bded1cc881a5a438a659a696c94bbfd553dd7ce14396872a41721f4ee35",
    "site/downloads/Sonata_1.0.8.app.tar.gz": "c0c4cb6f5e6113df0f3ae79c8b2e424cc62b68c9836bc41c93b09786b2d4bb44",
    "site/downloads/Sonata_1.0.7.dmg": "60365d98c7c0940b63434c94ec23ccbcc5c39e1c33c810b130c417305809d6c7",
    "site/downloads/Sonata_1.0.5.dmg": "26795aaa5b17ee84151fc3f349837d76c08e25bce34d518169bf9129c7201eff",
    "site/downloads/Sonata_1.0.0.dmg": "3eb221da36373f4adcec5b603285a2970c6c658b0421ae6bb8ba687fceb38044",
    "site/downloads/Sonata_1.0.3.dmg": "36c9fa179563da256ea9513e73fba3e105013c9546c1a6017f0fcd0c45798866",
    "site/downloads/Sonata_1.0.4.dmg": "1e80b869e9003f825cf268b7b1b42b919cae25b609133f40f72cb764687d53a9",
}
with tempfile.TemporaryDirectory(prefix="sonata-scan-") as temp:
    destination = Path(temp)
    for name in sorted(set(filter(None, names))):
        path = PurePosixPath(name)
        if path.is_absolute() or ".." in path.parts:
            failures.append(f"Unsafe path: {name}")
            continue
        example = path.name.startswith(".env.") and path.name.endswith(".example")
        if (denied.search(name) and not example and name not in public_artifacts) or any(part in {
            ".git", "node_modules", "target", ".tools", ".aws", ".codex", ".claude", ".agents",
            ".cache", ".worktrees", ".superpowers",
        } or part.endswith(".app") for part in path.parts) or name.startswith((
            "app/dist/", "app/src-tauri/gen/", "app/src-tauri/binaries/",
        )):
            failures.append(f"Private or generated file: {name}")
            continue
        if args.staged:
            entry = subprocess.check_output(["git", "ls-files", "--stage", "--", name]).split()
            if not entry or entry[0] not in (b"100644", b"100755"):
                failures.append(f"Unsupported staged entry: {name}")
                continue
            data = subprocess.check_output(["git", "show", f":{name}"])
        else:
            file = root / name
            if not file.exists():
                continue
            if file.is_symlink() or not file.is_file():
                failures.append(f"Unsupported source entry: {name}")
                continue
            data = file.read_bytes()
        if name in public_artifacts and hashlib.sha256(data).hexdigest() != public_artifacts[name]:
            failures.append(f"Public artifact checksum mismatch: {name}")
            continue
        out = destination / path
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_bytes(data)
    if failures:
        for failure in failures:
            print(failure)
        raise SystemExit(1)
    config = destination / ".gitleaks.toml"
    raise SystemExit(subprocess.call(["gitleaks", "dir", str(destination), "--config", str(config),
        "--gitleaks-ignore-path", str(destination / "absent-ignore"), "--ignore-gitleaks-allow",
        "--redact=100", "--no-banner", "--max-archive-depth=3", "--max-decode-depth=5"]))
