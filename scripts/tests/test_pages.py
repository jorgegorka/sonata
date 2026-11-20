import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import urllib.error

spec = importlib.util.spec_from_file_location("pages", Path(__file__).resolve().parents[1] / "pages.py")
pages = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pages)


class PagesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.files = {"index.html": b"<a>Download DMG</a>", "styles.css": b"a { color: red; }",
                      "downloads/Sonata.dmg": b"verified installer"}
        for name, content in self.files.items():
            path = self.directory / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        self.expected = pages.record(self.directory, "a" * 40)
        self.live = {**self.files, "": self.files["index.html"],
                     pages.MANIFEST: (self.directory / pages.MANIFEST).read_bytes()}
        self.urls = []

    def response(self, request, timeout):
        self.urls.append(request.full_url)
        prefix = "https://example.com/sonata/"
        self.assertTrue(request.full_url.startswith(prefix))
        path = request.full_url[len(prefix):]
        if path not in self.live:
            raise urllib.error.HTTPError(request.full_url, 404, "Not Found", {}, None)
        return io.BytesIO(self.live[path])

    def test_manifest_covers_publishable_files_and_can_be_regenerated(self):
        (self.directory / ".nojekyll").touch()
        (self.directory / ".hidden").mkdir()
        (self.directory / ".hidden/private.txt").write_text("excluded")
        manifest = pages.record(self.directory, "b" * 40)
        self.assertEqual(manifest, {"revision": "b" * 40, "files": {
            name: hashlib.sha256(content).hexdigest() for name, content in self.files.items()
        }})

    def test_matching_site_checks_project_root_and_every_file(self):
        with patch.object(pages.urllib.request, "urlopen", side_effect=self.response):
            pages.verify_once("https://example.com/sonata", self.expected)
        self.assertIn("https://example.com/sonata/", self.urls)
        self.assertEqual(len(self.urls), len(self.files) + 2)

    def test_old_revision_is_rejected_even_when_files_match(self):
        self.live[pages.MANIFEST] = json.dumps({**self.expected, "revision": "b" * 40}).encode()
        with patch.object(pages.urllib.request, "urlopen", side_effect=self.response):
            with self.assertRaisesRegex(ValueError, "manifest does not match"):
                pages.verify_once("https://example.com/sonata/", self.expected)

    def test_current_manifest_cannot_hide_stale_html_css_or_downloads(self):
        for name in self.live.keys() - {pages.MANIFEST}:
            with self.subTest(name=name), patch.dict(self.live, {name: b"stale content"}):
                with patch.object(pages.urllib.request, "urlopen", side_effect=self.response):
                    with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                        pages.verify_once("https://example.com/sonata/", self.expected)

    def test_missing_asset_retries_until_it_is_published(self):
        content = self.live.pop("styles.css")
        with patch.object(pages.urllib.request, "urlopen", side_effect=self.response):
            with patch.object(pages.time, "sleep", side_effect=lambda _: self.live.update({"styles.css": content})) as sleep:
                pages.verify("https://example.com/sonata/", self.expected)
        sleep.assert_called_once()

    def test_persistent_mismatch_fails_after_propagation_timeout(self):
        self.live["styles.css"] = b"stale CSS"
        with patch.object(pages.urllib.request, "urlopen", side_effect=self.response):
            with patch.object(pages.time, "monotonic", side_effect=[0, 1, 720]), patch.object(pages.time, "sleep"):
                with self.assertRaisesRegex(RuntimeError, "checksum mismatch: styles.css"):
                    pages.verify("https://example.com/sonata/", self.expected)


if __name__ == "__main__":
    unittest.main()
