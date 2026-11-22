# Changelog

## Unreleased

- Add projects with independent teams, boards, source directories, schedules, and workflows.
- Inherit the project’s default AI provider when creating bots through the UI or `sbot`.
- Add project selection, editing, archiving, and restoration while other projects continue working.
- Migrate existing installations to **Default**, preserving history and workspace paths, with a database backup.
- Scope configuration sharing to the selected project; keep skills shared and existing YAML files compatible.

## 1.0.2 — 2026-11-22

Omarchy package for x86_64. The signed Apple Silicon download on the homepage
remains 1.0.0. AppImage and DEB packages are still pending.

### Features

- Show completed tasks on the board. **Show done** reveals a Done column;
  **Hide done** puts it away.
- Restore a completed task from its card or drawer. Restoring brings back the
  sessions archived with it.

### Distribution

- Arch package `sonata-x86_64.pkg.tar.zst` on the Omarchy GitHub release.
  Install or upgrade it with `pacman -U`. Saved settings, bots, skills and history
  stay in place. Quit Sonata before upgrading.

## 1.0.1 — 2026-11-20

Omarchy package for x86_64. The signed Apple Silicon download on the homepage
remains 1.0.0. AppImage and DEB packages are still pending.

### Features

- Follow the live Omarchy theme, and repaint the window when that theme changes.
- Add **Export all…** to the Workflows page to save all workflows, bots and skills,
  including skill files, in one YAML file compatible with configuration import.

### Distribution

- Arch package `sonata-x86_64.pkg.tar.zst` on the Omarchy GitHub release.
  Install or upgrade it with `pacman -U`. Saved settings, bots, skills and history
  stay in place. Quit Sonata before upgrading.

## 1.0.0 — 2026-11-19

Initial Sonata release.

The published 1.0.0 download is the signed and notarized Apple Silicon macOS DMG.
Linux packaging is included in the source; downloadable Linux packages are pending.

### Features

- Run teams of local AI agents using Claude Code, Codex, Grok or OpenCode, with
  separate instructions, models, skills and working folders for each bot.
- Coordinate work through Master, direct chats, messages between bots and a board
  that follows requests through delegation and completion.
- Create branching workflows on a visual canvas or import them from plain text;
  schedule recurring requests while Sonata is running.
- Share skills across bots, archive and restore sessions, and import or export
  bots, skills and workflows.
- Browse pages beside a conversation, select an element and send feedback to a bot;
  inspect running CLI sessions in the live terminal.
- Limit messages between bots and detect usage limits, exited CLIs and blocked dialogs.
- Use the interface in English, Spanish, German or French.

### Distribution

- Apple Silicon macOS 11+ app and DMG packaging with Developer ID signing and
  Apple notarization.
- AppImage and DEB packaging for x86_64 and ARM64 Linux, including native menus,
  file operations and the browser element picker.
- Bundled herdr 0.9.3 and sbot helpers; users provide their preferred coding CLI.
- Terminal installer with versioned downloads, SHA-256 verification and protection
  against replacing a running app or downgrading it.
- Upgrades preserve saved settings, bots, skills and history. Quit Sonata before
  upgrading; running bot processes do not survive quitting.
- GitHub release automation and a homepage that advertises downloads after the
  complete platform release is published.
