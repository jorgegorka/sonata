# Sonata

Sonata is a desktop app for macOS and Linux for running a team of AI agents ("bots") on your own computer. Each bot
runs on a coding CLI you already use — **Claude Code, Codex, Grok or OpenCode** — with its own
name, instructions, model and skills. A built-in coordinator, **Master**, takes your requests,
delegates them to the right bots, checks their results and asks you before anything that needs
your approval.

> **Version:** 1.0.12. On Omarchy, install the x86_64 Arch package from the [1.0.7 release](https://github.com/jorgegorka/sonata/releases/tag/omarchy-1.0.7). macOS uses the signed and notarized [Apple Silicon DMG](https://jorgegorka.github.io/sonata/downloads/Sonata_1.0.12.dmg) ([SHA-256 checksum](https://jorgegorka.github.io/sonata/downloads/SHA256SUMS)), version 1.0.12. AppImage and DEB packages are coming soon. See the [changelog](CHANGELOG.md) for release notes.

> [!WARNING]
> Bots work on their own. Each CLI runs in its autonomous mode (for example Claude Code with
> `--dangerously-skip-permissions`, Codex with `--dangerously-bypass-approvals-and-sandbox`, Grok
> with `--always-approve`, OpenCode with `--auto`), so bots run commands and read and write files
> **without asking first, with the same access as your user account**. A bot's folder is where it
> works, not a sandbox. Only give bots work, skills and imported configurations you trust.

## Features

- **Guided setup** — describe your job and regular tasks, then choose tailored workflows with
  the bots and skills they need. Skip and return from Settings at any time.
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

## Getting started

On a new installation, Sonata guides you through setting up your first project:

1. Answer **What do you do?** and **Which tasks do you perform most often?** Any profession
   is welcome; describe the work you want help with and the tools or inputs you use.
2. Name your project, choose its work folder, and select an AI provider. The folder can
   contain documents, source code, or other project materials. Install and sign in to the
   selected provider's CLI before requesting suggestions; use **Check again** after signing in.
3. Click **Suggest workflows**. Master prepares up to three workflow cards. Review their
   steps, expected outputs, and required inputs or tools. Expand **Review instructions and
   skills** to inspect the instructions for each step, bot, and skill. Use **Edit answers and
   regenerate** if you want different suggestions.
4. Select the cards you want and click **Create selected workflows**. Sonata adds the selected
   workflows and the bots and skills they need, reusing suitable existing resources. Click
   **Open workflows** to give one a task when you are ready. Creation does not run workflows
   or enable schedules.

Your answers and unfinished proposals are saved locally for each project. Your answers are
sent to your configured AI provider when you request suggestions. New bots use the project's
default provider, and skills join the library shared across projects. Existing bots, workflows,
and skills are preserved.

Choose **Skip personalization** to continue with basic workspace setup and return later through
**Settings → Personalize workflows**. Existing installations keep their normal startup and can
open personalization from Settings for any active project. If generation fails, choose **Try
again**. If creation stops partway through, Sonata lists what was created; **Retry remaining
creation** finishes the remaining work without duplicates. If you have edited or removed a
resource from that proposal, generate a fresh proposal so your changes are preserved.

## Tasks and subtasks

Every task has a stable identity and can use a bot or its own workflow. Use **New task** or
**Add subtask** on any card. Subtasks have the same cards and statuses as their parents, and can
have children of their own. Conversations are execution history: a workflow's bot handoffs and
supporting conversations remain part of the same task.

**Pending** holds work that has not started, including tasks waiting for prerequisites or a
manual start. It is hidden by default; **Show Pending** displays it with its count. Ready tasks
start automatically unless held for manual start. Working tasks move to **Needs you** when input
is required. A bot must submit an explicit `sbot result` to finish; progress messages and going
idle do not finish work. **Finished** means execution ended, while **Done** is the user's archive.

Any bot, including Master, can plan subtasks within the user's request. Dependencies connect
siblings. Independent work runs in parallel; dependents start after their prerequisites are
accepted. Parent review is the default. A workflow step can enable automatic acceptance of
successful child results with its **Automatically accept successful subtask results** setting
(or `Subtasks: automatic` in the text format).

A workflow step that creates children waits for them and their reviews before advancing. An
early final result is saved across restarts. Child completion notifies its immediate parent;
the parent can review the work, continue its workflow, or finish. Failed or cancelled tasks
block dependents while independent work continues. There are no automatic retries.

Parent cards show progress over their direct children, including Pending and Done children,
with unsuccessful outcomes shown separately. The bar can reach 100% while the parent is still
reviewing. Open a card for its conversation, workflow controls, parent, children, requirements,
dependencies and result. Cancelling a parent cancels its unfinished descendants. Marking a
finished parent Done archives its descendants too. Restore preserves results without rerunning
work or accepting reviews. Follow-up work reopens the task and its ancestors without restarting
previously released dependents. Switching projects leaves background work running.

For agent integrations, `sbot orchestration start --key KEY` takes a JSON task graph on stdin.
Each task has a `key`, `title`, `instructions`, `acceptance_criteria`, `executor` (a bot `handle`
or workflow `name`), optional `depends_on` task keys, optional `files`, and optional `manual_start`.
Use `show ID` to read
results and the current revision, `update ID --revision N --key KEY` to submit `reviews`, `add`,
or `replace`, and `finish ID --revision N` with the final report on stdin. Stable operation keys
make identical retries safe; stale revisions are rejected. The parent conversation owns its
orchestration commands; child bots report with `sbot result`. Existing approval rules still apply to new
bots/workflows and actions outside the authorized scope.

The database stores this hierarchy in `work_items`, with session membership and dependencies
in separate tables. The older `tasks` table remains the recurring schedule configuration.
Upgrading preserves conversation IDs, provider context and history, and creates a
`sonata.db.work-items.bak` backup before migration.

## Projects

Use **Add project** on the main page to create a team with a name, source directory,
and default AI provider. Each project gets its own Master, bots, cards, schedules,
and workflows. The selector appears when more than one project is active, and
Sonata remembers your selection. Work continues in the other projects when you switch.
**Pause all bots** applies to every project.

New bots inherit the project's provider unless you choose another. Changing the
default does not change existing bots. Source-directory changes update managed
instructions on the next context start; custom instructions stay intact. Bot
workspaces remain separate from source directories, with independent memory and
inboxes for each project. The skill library is shared across projects.

**Manage projects** lets you edit, archive, and restore projects. Finish or stop
active, queued, and background work and unfinished workflows before archiving.
Archiving preserves history and completion states and suspends schedules. Restoration
resumes enabled schedules at their next future occurrence, without replaying missed runs.
Project deletion and moving cards or bots between projects are not supported.

Existing installations automatically become **Default**, using the existing directory
and Master's provider (Claude if there is no Master). Existing bot providers, IDs,
history, provider session IDs, and workspace paths are preserved. A database backup
named `sonata.db.projects.bak` is kept beside the database before this migration.
Legacy installations without a source directory can keep it unset; new projects
require one. Quit Sonata before upgrading.

## Sharing workflows and configuration

Share a useful workflow with a teammate, or take your complete bot setup to another computer.
Sonata saves both as YAML files (`.yaml` or `.yml`) that another Sonata installation can import.

| Contents | Project export | Individual workflow export |
| --- | --- | --- |
| Workflows | Every workflow in the selected project | The selected workflow |
| Bots | Every bot in the selected project, including its Master | The bots used by the selected workflow's steps |
| Skills | The shared skills assigned to the project’s bots | The skills assigned to those bots |
| Bot settings | Names, handles, colors, providers, models, instructions and skill assignments | The same settings for the included bots |
| Skill contents | `SKILL.md`, supporting files and executable permissions | The same contents for the included skills |

Workflow definitions include their instructions, steps, connections and canvas layout. Individual
exports follow the bot and skill assignments recorded in Sonata; mentioning another bot, skill or
file in instructions or scripts does not add it to the export.

### Export and import a project’s setup

1. Select the project, then open **Workflows** and choose **Export project…**. You can also use
   **Settings → Configuration → Export project**; both save the selected project’s configuration.
2. Choose where to save the YAML file. Keep it as a configuration backup or share it with the
   person who will use the setup.
3. On the receiving installation, select the destination project, then open **Settings → Configuration → Import into project** and select the file.
4. Review the preview, which lists new entries, entries already present and anything that cannot
   be imported. Choose **Add new** to add missing entries while preserving existing versions, or
   **Add and replace** to also update matching entries that differ. Unchanged entries are reused.

Importing merges the file into the selected project; it does not remove bots, skills or workflows
that are absent from the file. If the file or local setup changes after the preview, Sonata asks
you to review the updated preview before applying it. Skills remain shared: replacing a skill
can also affect bots in other projects. Existing YAML files remain compatible.

### Share and import one workflow

1. Open **Workflows** and click **Export…** beside the workflow you want to share.
2. Save the YAML file and send it to the recipient. Its bots and their assigned skills travel
   with it, so you do not need to export them separately.
3. The recipient opens **Workflows → Import workflow…** and selects the file. This also accepts a
   complete configuration export: if it contains several workflows, choose which one to import.
4. Review the workflow, bots and skills in the preview. New items are added and identical local
   items are reused. For conflicts, choose **Keep local** or **Replace** for each required item.
   To keep both versions of a workflow, choose **Import under a new name** and enter an unused name.
5. Resolve every required choice, then click **Import workflow**. Invalid or unresolved required
   items block the import until they are resolved.

For example, sharing a Builder → Reviewer workflow includes those two bots and their assigned
skills. Other workflows, unrelated bots and unassigned skills stay out of that file. When importing
from a complete export, the workflow importer likewise uses only the selected workflow and the
dependencies needed for your choices.

Keeping a local bot uses its existing instructions and skill assignments, which can change how
the imported workflow behaves. Replacing a shared bot or skill also affects its other users,
including subsequent messages in existing sessions. The preview identifies these shared uses.
Importing under a new workflow name still requires reviewing any shared bot and skill conflicts.

### What to prepare on the receiving computer

Configuration exports contain your bot and workflow definitions and skill files. Conversations,
workflow run history, project files in bot folders, recurring tasks, app preferences and provider
sign-ins are not included. Use a separate backup for that data.

Install and sign in to the coding CLIs the imported bots use, and make any required projects,
tools and external files available on the receiving computer. Review machine-specific paths in
your own instructions and scripts.

Review the file before sharing: bot instructions and bundled skill files are included, along with
any private information or secrets you put in them. Import only configurations from people you
trust, because skills can contain scripts your bots will run.

### Recover replaced configuration

Before replacing existing entries, Sonata saves their previous configuration to a YAML backup.
The import result reports the backup's filename in the Trash, or its saved path if Sonata could
not move it there. To restore those versions, recover the file from the Trash if needed, then use
**Settings → Configuration → Import…** and **Add and replace**.

This restores the replaced configuration; it does not remove new entries added by the import.
Check the import result for the entries written and any error if an import stops before finishing.

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
| `manifest.json`, `omarchy/`, `pkg/` | Omarchy bar plugin and Arch package |

## Installation

Download the signed, notarized macOS DMG from
[Sonata’s homepage](https://jorgegorka.github.io/sonata/). The 1.0.12 installer is stored in
`site/downloads/` and served by GitHub Pages. AppImage and DEB packages are pending. On
Omarchy, install the 1.0.7 Arch package with the steps in [Omarchy](#omarchy). Elsewhere, use the
source instructions below. Future automated desktop releases are published to
[GitHub Releases](https://github.com/jorgegorka/sonata/releases).

- **macOS:** Apple Silicon (M1 or newer), macOS 11+. Open the signed, notarized DMG and drag Sonata
  into Applications. Rust, Node.js and Xcode are not needed to run it.
- **Linux:** x86_64 or ARM64; Ubuntu 22.04 / Debian 12 or compatible, with a graphical desktop.
  The AppImage bundles the desktop libraries; the DEB installs its system dependencies.
  Check your architecture with `uname -m`. On Ubuntu/Debian install a downloaded DEB with
  `sudo apt install ./Sonata-linux-<architecture>.deb`.
- Install and sign in to at least one supported coding CLI: Claude Code, Codex, Grok or OpenCode.
  **herdr and sbot are included.**

Once packages are published to GitHub Releases, the terminal installer below can install and
update them. For the current Pages-hosted 1.0.12 release, use the DMG download above.

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
curl -fsSL https://jorgegorka.github.io/sonata/install.sh | bash -s -- --version 1.0.7
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


### Automatic updates

Version 1.0.8 is the first release with this feature. Upgrade older macOS versions
manually using the DMG; subsequent versions can be offered inside Sonata.

Settings → **Automatic updates** is enabled by default. Sonata checks once when
it starts and offers newer stable versions for your installation. It asks before
downloading; after verification, **Restart and install** asks you to stop running
bot work across all projects. Saved data and configuration are preserved.
**Later** dismisses the offer until the next launch. Turning the checkbox on takes
effect at the next launch. Offline startup does not show an error.

macOS, AppImage, and DEB installations support in-app installation. DEB updates
may require administrator authorization. Omarchy packages and Linux installations
made through the terminal installer show matching update instructions instead.
Finish work and quit Sonata before following those instructions.

Versions before this feature need one manual upgrade. Developer builds do not
check for updates. See [the release guide](docs/releases.md#automatic-update-feeds)
for signing, feed publication, and validation.

## Omarchy

Sonata 1.0.7 for Omarchy is an Arch package. It follows the Omarchy theme and
updates the window when you change that theme. Sonata is absent from the Install
menu (Package, AI, Service, Editor), so searching there will not find it.

### Install the package

1. Open a terminal.
2. Download the package for this computer.

   ```sh
   curl -fLo /tmp/sonata.pkg.tar.zst \
     "https://github.com/jorgegorka/sonata/releases/latest/download/sonata-$(uname -m).pkg.tar.zst"
   ```

3. Install it. Pacman asks for your password. The same command upgrades an older Sonata package.

   ```sh
   sudo pacman -U /tmp/sonata.pkg.tar.zst
   ```

4. Press Super+Space, type Sonata, and open it. The command on disk is `/usr/bin/sonata`.

Quit Sonata before you upgrade. Your database and bot folders stay where they are.
This release publishes the x86_64 package. The macOS app is the signed DMG from the homepage.

### Build it from a checkout

The script `omarchy/install-app.sh` builds the same Arch package from a checkout
and installs it with pacman. Use it when you want this source tree rather than the published package.

1. Open a terminal.
2. Clone this repository and enter it. Skip the clone if you already have the checkout.

   ```sh
   git clone https://github.com/jorgegorka/sonata.git
   cd sonata
   ```

3. Run the installer as your own user. It asks for your password when the package is ready to install.

   ```sh
   omarchy/install-app.sh
   ```

4. Wait for the build. The script installs the compilers and libraries it needs, compiles Sonata in a temporary copy of the tree, and then runs `sudo pacman -U` on the finished package. The first build takes a while.
5. When the script prints that Sonata is installed, press Super+Space, type Sonata, and open it.

If pacman already has this exact package version, the script prints `Sonata <version> is already installed.` and stops. A newer checkout upgrades the installed package.

### Remove the Omarchy package

Quit Sonata, then:

```sh
sudo pacman -Rns sonata
```

That removes the application. Your database and bot folders stay where they are.

### Bar button

The repository also includes an optional bar button, plugin id `io.github.jorgegorka.sonata`. It focuses the Sonata window, or starts `/usr/bin/sonata` after the package above is installed. It is not listed on the [Omarchy plugin marketplace](https://plugins.omarchy.org/) yet.

```sh
omarchy plugin add https://github.com/jorgegorka/sonata.git --enable
omarchy bar move io.github.jorgegorka.sonata --section right
```

Check the manifest with `omarchy plugin validate .`. Remove the button with `omarchy plugin remove io.github.jorgegorka.sonata`. Removing the button leaves the pacman package in place.

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

Setup downloads the checksum-pinned herdr 0.9.5 for your architecture, installs frontend
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

For 1.0.12, these create `target/release/bundle/macos/Sonata.app` and
`target/release/bundle/dmg/Sonata_1.0.12.dmg`. Both commands sign, submit to Apple and staple the
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

To export your setup or share one workflow, see
[Sharing workflows and configuration](#sharing-workflows-and-configuration).

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
that folder or its workflow changes on `master`. It also runs after publishing releases and can be run manually from **Actions**.
When the latest GitHub Release has no desktop packages, deployment verifies the DMG in
`site/downloads/` against its SHA-256 manifest and generates its download link instead.
Each deployment records the checked-out `master` commit and generated file checksums in
`deployment.json`. After publishing, the workflow checks the public manifest, homepage,
and every published file (including downloads). It retries for up to 12 minutes to allow
cache propagation and fails if the public content still differs from the uploaded artifact.
If deployment fails before publishing, the previous site remains live; start a fresh run of
**Deploy homepage to GitHub Pages** on `master` from **Actions**, then wait for verification to pass.
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
