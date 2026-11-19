#!/usr/bin/env python3
"""Exercise the packaged GTK app and isolated picker through native WebKit WebDriver."""
import argparse
from contextlib import contextmanager
import http.server
import json
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request


def eventually(check, description, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(0.1)
    raise RuntimeError(f"Timed out: {description}")


class Driver:
    def __init__(self, port):
        self.base = f"http://127.0.0.1:{port}"
        self.session = ""

    def request(self, method, path, body=None):
        data = json.dumps(body).encode() if body is not None else None
        request = urllib.request.Request(self.base + path, data=data, method=method, headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(request, timeout=45) as response:
                result = json.load(response)["value"]
        except urllib.error.HTTPError as error:
            raise RuntimeError(error.read().decode()) from error
        if isinstance(result, dict) and "error" in result:
            raise RuntimeError(result)
        return result

    def command(self, method, path, body=None):
        return self.request(method, f"/session/{self.session}{path}", body)

    def script(self, source, *args):
        return self.command("POST", "/execute/sync", {"script": source, "args": list(args)})

    def invoke(self, command, args=None):
        result = self.command("POST", "/execute/async", {
            "script": "const done=arguments[2]; window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(value=>done({ok:true,value}),error=>done({ok:false,error}));",
            "args": [command, args or {}],
        })
        if not result["ok"]:
            raise RuntimeError(f"{command}: {result}")
        return result.get("value")

    def switch(self, handle):
        self.command("POST", "/window", {"handle": handle})

    def matching_view(self, predicate):
        for handle in self.command("GET", "/window/handles"):
            self.switch(handle)
            if self.script(predicate):
                return handle
        return None

    def click(self, x, y):
        self.command("POST", "/actions", {"actions": [{"type": "pointer", "id": "mouse", "parameters": {"pointerType": "mouse"}, "actions": [
            {"type": "pointerMove", "duration": 0, "origin": "viewport", "x": round(x), "y": round(y)},
            {"type": "pointerDown", "button": 0}, {"type": "pointerUp", "button": 0},
        ]}]})

    def element(self, selector):
        result = self.command("POST", "/element", {"using": "css selector", "value": selector})
        return result["element-6066-11e4-a52e-4f735466cecf"]


@contextmanager
def fixture():
    class Page(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_): pass

        def do_GET(self):
            body = b'''<!doctype html><title>Sonata picker fixture</title>
<style>body{margin:0;padding:40px;height:2000px}#target{width:250px;height:80px;background:#ddd}</style>
<button id="target">Native picker target</button><iframe srcdoc="<p>Isolated frame</p>"></iframe>'''
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Page)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try: yield f"http://127.0.0.1:{server.server_port}/"
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def picker(driver, main):
    # Observe the real native submission in the trusted app, without starting an agent.
    callback = driver.script("window.__packagingPick=null; return window.__TAURI_INTERNALS__.transformCallback(e=>window.__packagingPick=e.payload)")
    driver.invoke("plugin:event|listen", {"event": "browser-pick", "target": {"kind": "Any"}, "handler": callback})
    with fixture() as url:
        rect = {"x": 450, "y": 60, "width": 600, "height": 600, "viewport_width": 1100, "viewport_height": 720}
        driver.invoke("browser_open", {"url": url, "rect": rect})
        panel = eventually(lambda: driver.matching_view("return document.title === 'Sonata picker fixture'"), "native browser panel")
        assert driver.script("return typeof window.__sonataPost === 'undefined' && !window.webkit?.messageHandlers?.sonataPick"), "Page world can reach picker bridge"
        assert driver.script("return !document.querySelector('iframe').contentWindow.webkit?.messageHandlers?.sonataPick"), "Frame can reach picker bridge"
        driver.switch(main)
        strings = {"hint": "Select an element", "placeholder": "Comment", "keys": "Escape to cancel", "send": "Send", "sending": "Sending", "failed": "Failed", "noTarget": "Choose a chat", "tooLong": "Too long"}
        driver.invoke("browser_pick_start", {"strings": strings, "target": "Packaging QA"})
        driver.switch(panel)
        time.sleep(0.3)
        # A synthetic DOM click must not select anything; real pointer input must.
        driver.script("document.querySelector('#target').click()")
        time.sleep(0.2)
        assert len(driver.command("GET", "/window/handles")) == 2, "Synthetic click opened the picker"
        x, y = driver.script("const r=document.querySelector('#target').getBoundingClientRect(); return [r.x+r.width/2,r.y+r.height/2]")
        driver.click(x, y)
        box = eventually(lambda: driver.matching_view("return location.pathname.endsWith('pickbox.html')"), "trusted comment webview")
        eventually(lambda: driver.script("return document.querySelector('#target').textContent.includes('Packaging QA')"), "picker target")
        driver.command("POST", f"/element/{driver.element('#comment')}/value", {"text": "\ue00c"})
        driver.switch(main)
        try:
            driver.invoke("browser_pick_submit", {"comment": "Cancelled selection"})
        except RuntimeError as error:
            assert "no element is picked" in str(error), error
        else:
            raise AssertionError("Escape left an element selected")
        for address in [url, url + "second", url]:
            driver.script("window.__packagingPick=null")
            driver.invoke("browser_navigate", {"address": address, "rect": rect})
            driver.switch(panel)
            eventually(lambda: driver.script("return location.href === arguments[0] && document.readyState === 'complete'", address), "navigation")
            time.sleep(0.3)
            driver.click(x, y)
            driver.script("window.scrollTo(0,24)")
            driver.switch(box)
            field = driver.element('#comment')
            driver.command("POST", f"/element/{field}/clear", {})
            comment = "Packaged Linux native picker check"
            driver.command("POST", f"/element/{field}/value", {"text": comment})
            driver.command("POST", f"/element/{driver.element('#send')}/click", {})
            driver.switch(main)
            picked = eventually(lambda: driver.script("return window.__packagingPick"), "native picker submission")
            assert picked["comment"] == comment and picked["element"]["selector"] == "#target", picked
            assert picked["element"]["url"] == address and picked["element"]["text"] == "Native picker target", picked
            driver.invoke("browser_pick_close")
        driver.invoke("browser_pick_stop")
        driver.invoke("browser_close")
        print("Native picker passed: isolated page/frame bridge, trusted selection, scrolling, Escape, navigation and comment submission", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    parser.add_argument("--picker", action="store_true")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="sonata-native-") as temp:
        root = Path(temp)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        env = {**os.environ, "SONATA_DATA_DIR": str(root / "data"), "SONATA_BOTS_ROOT": str(root / "bots"), "TAURI_WEBVIEW_AUTOMATION": "true", "LANG": "C.UTF-8", "LANGUAGE": "en"}
        env.pop("SONATA_HERDR_BIN", None)
        if os.environ.get("SONATA_CI_CONTAINER") == "1":
            env["WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS"] = "1"
        with (root / "driver.log").open("w+") as log:
            process = subprocess.Popen(["dbus-run-session", "--", "xvfb-run", "-a", "WebKitWebDriver", f"--port={port}"], env=env, stdout=log, stderr=log, start_new_session=True)
            driver = Driver(port)
            try:
                def ready():
                    try: return driver.request("GET", "/status")["ready"]
                    except urllib.error.URLError: return False
                eventually(ready, "WebKitWebDriver")
                session = driver.request("POST", "/session", {"capabilities": {"alwaysMatch": {"webkitgtk:browserOptions": {"binary": str(args.executable.resolve()), "args": []}}}})
                driver.session = session["sessionId"]
                main_view = driver.command("GET", "/window")
                eventually(lambda: driver.script("return document.body.innerText.includes('Welcome to Sonata')"), "setup UI")
                health = driver.invoke("herdr_status")
                pin = Path(__file__).with_name("herdr-runtime.sh").read_text()
                expected_version = re.search(r"HERDR_VERSION=([0-9.]+)", pin).group(1)
                assert health["error"] is None and health["version"] == expected_version, health
                assert "/runtime/herdr" in health["path"], health
                print(f"Packaged setup and herdr health passed: {args.executable}", flush=True)
                if args.picker:
                    picker(driver, main_view)
            except Exception:
                log.flush()
                log.seek(0)
                print(log.read(), flush=True)
                raise
            finally:
                if driver.session:
                    try: driver.command("DELETE", "")
                    except (RuntimeError, urllib.error.URLError): pass
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                    try: process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait()


if __name__ == "__main__":
    main()
