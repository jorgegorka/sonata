# Sonata

Sonata is a desktop app for macOS and Linux for running a team of AI agents ("bots") on your own computer. Each bot
runs on a coding CLI you already use — **Claude Code, Codex, Grok or OpenCode** — with its own
name, instructions, model and skills. A built-in coordinator, **Master**, takes your requests,
delegates them to the right bots, checks their results and asks you before anything that needs
your approval.

> **Version:** 1.0.0. The signed and notarized [Apple Silicon DMG](https://github.com/jorgegorka/sonata/releases/download/v1.0.0/Sonata-macos-aarch64.dmg) is available. Linux packages are coming soon; Linux users can build from source. See the [changelog](CHANGELOG.md) for release notes.

> [!WARNING]
> Bots work on their own. Each CLI runs in its autonomous mode (for example Claude Code with
> `--dangerously-skip-permissions`, Codex with `--dangerously-bypass-approvals-and-sandbox`, Grok
> with `--always-approve`, OpenCode with `--auto`), so bots run commands and read and write files
> **without asking first, with the same access as your user account**. A bot's folder is where it
> works, not a sandbox. Only give bots work, skills and imported configurations you trust.

## Features

- **Master** — a coordinator bot that discovers your bots, skills and workflows, routes requests,
  verifies results and proposes new bots and skills for you to approve.
- **Bots** — each with a handle, color, provider, model, instructions and skills, working in its
  own folder. Chat with any bot directly, or let them message each other.
- **Board** — follow every request from the moment you send it to its result, including the work
  bots handed to each other on the way.
- **Skills** — a shared library of `SKILL.md` folders, copied into each bot that uses them.
- **Workflows** — hand a request from bot to bot, one step at a time, branching on each step's
  outcome until it reaches `Done`. Build them on a visual canvas or in a plain-text format.
- **Recurring tasks** — give a bot the same request in a fresh session every few days, at a time
  you choose, while Sonata is running.
- **Built-in browser** — open pages next to the chat, pick an element on the page and send a
  comment about it to a bot.
- **Live terminal** — watch any bot's CLI session as it works.
- **Safeguards** — a limit on messages between bots per request, so two bots cannot keep each other
  busy forever; usage-limit detection; a watchdog that notices a CLI that exited or is stuck on a
  dialog.
- **Archive, import and export** — archive and restore sessions; save your bots, skills and
  workflows to a file and load them on another computer.
- English, Spanish, German and French interface. Bots write to you in your language.

## How it works

```text
Sonata app (Tauri + React)
      │
sonata-core engine ─────────── SQLite (sonata.db)
      │  herdr-client: NDJSON over a Unix socket
herdr (bundled, headless terminal sessions)
      ├── bot pane: claude ─┐
      ├── bot pane: codex   ├── bots call `sbot` ──► bridge.sock ──► engine
      └── bot pane: …      ─┘
```

Sonata runs each bot's CLI in a private, headless [herdr](https://github.com/herdrdev/herdr)
session. Bots talk back to the app with `sbot`, a small CLI bundled with Sonata
(`sbot reply`, `sbot send @handle`, `sbot ask`, `sbot result`, …), over a Unix socket. Sonata
reads each pane's screen to know when a bot is working, idle, waiting or out of usage.

Everything stays on your computer: the database, logs and skill library live in the data folder, and
each bot's files live in its own folder. Sonata has no telemetry. Each CLI talks to its own
provider with the account you signed in with.

## Repository layout

| Path | What it is |
| --- | --- |
| `crates/sonata-core` | Domain model, engine, SQLite store, workflows, scheduling, provider adapters |
| `crates/herdr-client` | Typed client for herdr's Unix-socket API |
| `crates/bridge-proto` | Wire types shared by `sbot` and the app |
| `crates/sbot` | The CLI bots use to talk to the app and to each other |
| `app/src-tauri` | Tauri shell: windows, menus, tray, notifications, browser panel |
| `app/src` | React interface |
| `app/locales` | Interface translations |
| `scripts` | Setup and secret-scanning scripts |
| `site` | Standalone homepage published with GitHub Pages |

## Installation

Download a stable build from [Sonata’s homepage](https://jorgegorka.github.io/sonata/) or
[GitHub Releases](https://github.com/jorgegorka/sonata/releases). Version 1.0.0 provides a
signed, notarized macOS DMG. Linux packages are pending; use the source instructions below.

- **macOS:** Apple Silicon (M1 or newer), macOS 11+. Open the signed, notarized DMG and drag Sonata
  into Applications. Rust, Node.js and Xcode are not needed to run it.
- **Linux:** x86_64 or ARM64; Ubuntu 22.04 / Debian 12 or compatible, with a graphical desktop.
  The AppImage bundles the desktop libraries; the DEB installs its system dependencies.
  Check your architecture with `uname -m`. On Ubuntu/Debian install a downloaded DEB with
  `sudo apt install ./Sonata-linux-<architecture>.deb`.
- Install and sign in to at least one supported coding CLI: Claude Code, Codex, Grok or OpenCode.
  **herdr and sbot are included.**

You can also install from a terminal on platforms with a published package (currently macOS):

```sh
curl -fsSL https://jorgegorka.github.io/sonata/install.sh | bash
```

[Inspect the installer](site/install.sh) before running it. It detects your platform, downloads
one stable release, verifies its SHA-256 checksum, and installs the app. On Linux it extracts
the AppImage so this installation route does not need FUSE, creates `~/.local/bin/sonata`, and
adds a desktop launcher. Install `xdg-utils` if your distribution does not already provide it.
A directly downloaded AppImage needs executable permission and FUSE 2; alternatively use the
terminal installer above or run it with `--appimage-extract-and-run`.

### Updates and custom installations

Finish your work and **Quit Sonata** before updating; closing its window keeps bots working
(macOS hides it, Linux minimizes it). Quitting stops its herdr session. Rerun the installer,
replace the app from a new DMG, or install the newer DEB. The terminal installer refuses
running-app replacement and downgrades. It never edits your database, bot folders or skills.
DEB installations stay managed by the package manager.

```sh
# Choose a stable release or a user-writable Mac destination.
curl -fsSL https://jorgegorka.github.io/sonata/install.sh | bash -s -- --version 1.0.0
curl -fsSL https://jorgegorka.github.io/sonata/install.sh | bash -s -- --dest "$HOME/Applications/Sonata.app"
```

`--dest` is the full `.app` path on Mac or the application directory on Linux. Use the same
custom destination for later updates. The Linux installer also honors `XDG_DATA_HOME` for
launchers/icons and `SONATA_INSTALL_PREFIX` for its default `opt/sonata` and `bin` directories
(default `~/.local`). Release notes and `SHA256SUMS` accompany every download.

### Uninstall

Quit Sonata first. On Mac, move `Sonata.app` to the Trash. For a DEB, use your package manager.
For a terminal installation on Linux, remove `~/.local/opt/sonata`, `~/.local/bin/sonata`,
`~/.local/share/applications/io.sonata.desktop.desktop`, and
`~/.local/share/icons/hicolor/*/apps/io.sonata.desktop.png`, adjusting custom paths.
Removing the application preserves your data. Remove its data folder and bot folders separately
only if you also want to delete your saved work.

## Build from source

You need stable Rust (CI uses 1.100.0), Node.js 20.19+, 22.12+ or 24+, and platform build tools.
On Mac, install Xcode Command Line Tools. On Ubuntu/Debian:

```sh
sudo apt install build-essential pkg-config curl libwebkit2gtk-4.1-dev libgtk-3-dev \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev patchelf \
  libfuse2 xdg-utils
```

```sh
git clone https://github.com/jorgegorka/sonata.git
cd sonata
scripts/setup.sh
```

Setup downloads the checksum-pinned herdr 0.9.3 for your architecture, installs frontend
dependencies, and enables the repository's pre-commit hook. Committing requires
[gitleaks](https://github.com/gitleaks/gitleaks) (`brew install gitleaks` on Mac).

### Run in development

```sh
cargo build -p sbot
cd app
SONATA_HERDR_BIN="$PWD/../.tools/herdr/herdr" \
SONATA_DATA_DIR="$PWD/../.tools/dev-data" \
SONATA_BOTS_ROOT="$PWD/../.tools/dev-bots" \
npm run tauri dev
```

These data overrides keep development work separate from an installed app. Rebuild `sbot`
after changing it. Only one instance can use a data folder at a time.

### Build packages

```sh
cd app
npm run bundle:unsigned
```

On Mac this creates `target/release/bundle/macos/Sonata.app`; run `npm run dmg:unsigned` next
to create the DMG. On Linux it creates AppImage and DEB files in `target/release/bundle/`.
Linux helpers live in the application's private resources, without installing a global herdr.
Unsigned builds are for local development; public Mac releases require signing and notarization.

### Sign and notarize a Mac release

Use a **Developer ID Application** certificate and its private key from your Apple Developer
team. The same certificate can sign multiple apps from that team. Notarization also needs
either an App Store Connect **team API key** with **Developer** access, or your Apple Account
email, an app-specific password and Team ID. Those credentials can also be reused across apps;
each release still needs its own signing and notarization submission.

With the signing identity installed in your Mac's keychain and the Apple environment variables
configured, run:

```sh
npm --prefix app run bundle
npm --prefix app run dmg
```

For 1.0.0, these create `target/release/bundle/macos/Sonata.app` and
`target/release/bundle/dmg/Sonata_1.0.0.dmg`. Both commands sign, submit to Apple and staple the
accepted notarization ticket when the required credentials are present.

See [Releasing Sonata](docs/releases.md) for the exact credential variables, verification
commands, CI setup and publishing. Keep certificates, private keys and passwords out of Git.

## Testing

```sh
cargo test --workspace
cd app && npm test
```

Tests run against a fake runtime and need no CLI or network. Live tests drive real CLIs through
herdr and use your accounts' usage, so they are opt-in:

```sh
SONATA_HERDR_BIN="$PWD/.tools/herdr/herdr" \
  cargo test -p sonata-core --test herdr_live -- --ignored
```

They need the CLIs they name to be installed and signed in. `SONATA_LIVE_OPENCODE_MODEL` picks
the OpenCode model they use. herdr-client's live tests run whenever `SONATA_HERDR_BIN` is set.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `SONATA_DATA_DIR` | Mac: `~/Library/Application Support/Sonata`; Linux: `$XDG_DATA_HOME/sonata` or `~/.local/share/sonata` | Database, logs, skill library |
| `SONATA_BOTS_ROOT` | the folder chosen at setup, else `~/Sonata/Bots` | Where bots keep their files |
| `SONATA_HERDR_BIN` | the packaged herdr runtime | herdr binary for development runs |

## Homepage (GitHub Pages)

The homepage lives in `site/`. It is plain HTML, CSS and JavaScript, with no build step or
dependencies on the desktop app. Preview it locally from the repository root:

```sh
python3 -m http.server 8080 --bind 127.0.0.1 --directory site
```

Open <http://localhost:8080>. The icon assets in `site/assets/` are copies of `app/app-icon.svg`
and `app/app-icon.png`; refresh both copies when the app icon changes.

To enable publishing, open the repository's **Settings → Pages** and select **GitHub Actions**
as the source. The `Deploy homepage to GitHub Pages` workflow generates links for available platform packages in published releases and publishes `site/` whenever
that folder or its workflow changes on `master`. It also runs after publishing releases and can be run manually from **Actions**. Without a release, the page links to source instructions and the releases list.
See [GitHub's custom workflow documentation](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages).

After the first successful deployment, the homepage will be available at
<https://jorgegorka.github.io/sonata/>. If hosting the site under another repository or domain,
update the canonical and Open Graph URLs in `site/index.html`.

## Contributing

Issues and pull requests are welcome. Before you send changes:

- run both test suites above;
- format Rust code with `cargo fmt` (see `rustfmt.toml`);
- keep secrets out of the repository: the pre-commit hook runs `scripts/check-secrets.sh`, which
  scans the files you are committing with gitleaks. Run it without arguments to also scan
  untracked files that are not ignored.

## License

Sonata is released under the [MIT License](LICENSE). The app bundles herdr, which is licensed
under the Apache License 2.0; its license ships with the app
(`app/src-tauri/licenses/herdr-LICENSE.txt`).
