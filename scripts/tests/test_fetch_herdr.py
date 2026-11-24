import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
TARGETS = {
    "aarch64-apple-darwin": "herdr-macos-aarch64",
    "x86_64-unknown-linux-gnu": "herdr-linux-x86_64",
    "aarch64-unknown-linux-gnu": "herdr-linux-aarch64",
}


class FetchHerdrTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="sonata runtime ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        scripts = self.root / "scripts"
        scripts.mkdir()
        shutil.copy2(ROOT / "scripts/fetch-herdr.sh", scripts)
        self.payload = self.root / "release-binary"
        self.payload.write_bytes(b"#!/bin/sh\nprintf 'verified runtime\\n'\n")
        pin = (ROOT / "scripts/herdr-runtime.sh").read_text()
        self.version = re.search(r"HERDR_VERSION=([0-9.]+)", pin).group(1)
        digest = hashlib.sha256(self.payload.read_bytes()).hexdigest()
        (scripts / "herdr-runtime.sh").write_text(
            re.sub(r"HERDR_SHA256=[a-f0-9]{64}", f"HERDR_SHA256={digest}", pin)
        )
        self.requests = self.root / "requests"
        shim = self.root / "bin"
        shim.mkdir()
        curl = shim / "curl"
        curl.write_text("""#!/bin/sh
set -eu
printf '%s\\n' "$@" >> "$SONATA_TEST_REQUESTS"
while [ "$#" -gt 0 ]; do
  if [ "$1" = -o ]; then cp "$SONATA_TEST_PAYLOAD" "$2"; exit 0; fi
  shift
done
exit 1
""")
        curl.chmod(0o755)
        self.env = dict(os.environ)
        self.env.pop("SONATA_HERDR_BIN", None)
        self.env.update({
            "PATH": f"{shim}{os.pathsep}{self.env['PATH']}",
            "SONATA_TEST_REQUESTS": str(self.requests),
            "SONATA_TEST_PAYLOAD": str(self.payload),
        })

    def fetch(self, target="aarch64-apple-darwin", override=None):
        env = dict(self.env)
        if override is not None:
            env["SONATA_HERDR_BIN"] = str(override)
        return subprocess.run(
            ["sh", str(self.root / "scripts/fetch-herdr.sh"), target],
            cwd=self.root, env=env, text=True, capture_output=True, timeout=10,
        )

    def cache(self, target="aarch64-apple-darwin"):
        return self.root / ".tools/herdr" / self.version / target / "herdr"

    def test_each_target_downloads_verified_binary_and_reuses_cache(self):
        for target, asset in TARGETS.items():
            with self.subTest(target=target):
                result = self.fetch(target)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), str(self.cache(target)))
                self.assertEqual(self.cache(target).read_bytes(), self.payload.read_bytes())
                self.assertTrue(os.access(self.cache(target), os.X_OK))
                requests = self.requests.read_text()
                self.assertIn(
                    f"https://github.com/herdrdev/herdr/releases/download/v{self.version}/{asset}",
                    requests,
                )
                self.assertEqual(self.fetch(target).returncode, 0)
                self.assertEqual(self.requests.read_text(), requests, "cached fetch must not download")

    def test_upgrade_leaves_previous_and_unversioned_caches_untouched(self):
        old_caches = [
            self.root / ".tools/herdr/aarch64-apple-darwin/herdr",
            self.root / ".tools/herdr/0.9.3/aarch64-apple-darwin/herdr",
        ]
        for old in old_caches:
            old.parent.mkdir(parents=True)
            old.write_bytes(b"previous runtime")
        result = self.fetch()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), str(self.cache()))
        for old in old_caches:
            self.assertEqual(old.read_bytes(), b"previous runtime")

    def test_corrupt_download_is_not_cached_and_temporary_file_is_removed(self):
        self.payload.write_bytes(b"corrupt release")
        result = self.fetch()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Runtime checksum mismatch", result.stderr)
        self.assertFalse(self.cache().exists())
        self.assertEqual(list(self.cache().parent.iterdir()), [])

    def test_corrupt_cache_is_rejected_without_overwriting_or_downloading(self):
        self.cache().parent.mkdir(parents=True)
        self.cache().write_bytes(b"corrupt cache")
        result = self.fetch()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Runtime checksum mismatch", result.stderr)
        self.assertEqual(self.cache().read_bytes(), b"corrupt cache")
        self.assertFalse(self.requests.exists())

    def test_override_is_verified_and_never_replaced_or_downloaded(self):
        override = self.root / "custom runtime"
        for payload, succeeds in [(self.payload.read_bytes(), True), (b"wrong version", False), (None, False)]:
            with self.subTest(payload=payload):
                if payload is None:
                    override.unlink()
                else:
                    override.write_bytes(payload)
                result = self.fetch(override=override)
                self.assertEqual(result.returncode == 0, succeeds, result.stderr)
                if succeeds:
                    self.assertEqual(result.stdout.strip(), str(override))
                elif payload is None:
                    self.assertIn("SONATA_HERDR_BIN does not exist", result.stderr)
                else:
                    self.assertIn("Runtime checksum mismatch", result.stderr)
                if payload is not None:
                    self.assertEqual(override.read_bytes(), payload)
                else:
                    self.assertFalse(override.exists())
                self.assertFalse(self.requests.exists())
                self.assertFalse(self.cache().exists())

    def test_unsupported_target_fails_before_download(self):
        result = self.fetch("x86_64-pc-windows-msvc")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Unsupported Sonata target", result.stderr)
        self.assertFalse(self.requests.exists())


if __name__ == "__main__":
    unittest.main()
