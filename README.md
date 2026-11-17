# Sonata

Sonata is a macOS desktop app for running a team of AI agents ("bots") on your own Mac. Each bot
runs on a coding CLI you already use — **Claude Code, Codex, Grok or OpenCode** — with its own
name, instructions, model and skills. A built-in coordinator, **Master**, takes your requests,
delegates them to the right bots, checks their results and asks you before anything that needs
your approval.

> **Status:** early development (0.1.0). Apple Silicon Macs only.

> [!WARNING]
> Bots work on their own. Each CLI runs in its autonomous mode (for example Claude Code with
> `--dangerously-skip-permissions`, Codex with `--dangerously-bypass-approvals-and-sandbox`, Grok
> with `--always-approve`, OpenCode with `--auto`), so bots run commands and read and write files
> **without asking first, with the same access as your macOS user**. A bot's folder is where it
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
  workflows to a file and load them on another Mac.
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

Everything stays on your Mac: the database, logs and skill library live in the data folder, and
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

## Requirements

- An Apple Silicon Mac (the bundled herdr runtime is pinned to its macOS arm64 release)
- [Rust](https://rustup.rs) (stable) and the Xcode Command Line Tools
- Node.js 20.19+, 22.12+ or 24+
- At least one of Claude Code, Codex, Grok or OpenCode, installed and signed in
- [gitleaks](https://github.com/gitleaks/gitleaks) on your `PATH` to commit (`brew install gitleaks`)

## Getting started

```sh
scripts/setup.sh
```

This downloads the pinned herdr 0.9.3 to `.tools/herdr/` and checks its SHA-256, installs the
frontend dependencies, and turns on the repository's pre-commit hook.

### Run in development

```sh
cargo build -p sbot            # the app looks for sbot next to its own executable
cd app
SONATA_HERDR_BIN="$PWD/../.tools/herdr/herdr" \
SONATA_DATA_DIR="$HOME/Library/Application Support/Sonata-dev" \
SONATA_BOTS_ROOT="$HOME/Sonata/Bots-dev" \
npm run tauri dev
```

The two `SONATA_*_DIR`/`ROOT` variables are optional. They keep development data apart from an
installed copy of Sonata; only one app instance can use a data folder at a time. Rebuild `sbot`
after changing it.

### Build the app

```sh
cd app
npm run bundle:unsigned   # target/release/bundle/macos/Sonata.app
npm run dmg:unsigned      # target/release/bundle/dmg/Sonata_<version>.dmg (after the bundle)
```

`npm run bundle` and `npm run dmg` sign the app and the disk image when `APPLE_SIGNING_IDENTITY`
is set, and notarize them when either `APPLE_ID` + `APPLE_PASSWORD` + `APPLE_TEAM_ID` or
`APPLE_API_KEY` + `APPLE_API_ISSUER` + `APPLE_API_KEY_PATH` are set too. Credentials are only
read from the environment; never commit them.

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
| `SONATA_DATA_DIR` | `~/Library/Application Support/Sonata` | Database, logs, skill library |
| `SONATA_BOTS_ROOT` | the folder chosen at setup, else `~/Sonata/Bots` | Where bots keep their files |
| `SONATA_HERDR_BIN` | the `herdr` next to the app's executable | herdr binary for development runs |

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
