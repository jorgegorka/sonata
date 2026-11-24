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
