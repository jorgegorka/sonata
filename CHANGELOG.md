# Changelog

## Unreleased

## 1.0.12 — 2026-11-28

Signed and notarized Apple Silicon macOS DMG and signed automatic update archive.
The Omarchy package remains 1.0.7; AppImage and DEB packages are still pending.

### Task creation

- Replace the task-creation drawer with a centered, Linear-style dialog: an
  optional title, open description area, compact bot/workflow and start controls,
  expandable subtask dependencies, and a fixed attachment/action bar.
- Use the same dialog from the board, bot shortcuts, workflow Run, newly created
  bots, and subtasks. Preserve file picking and drag-and-drop, automatic titles,
  manual starts, dependencies, and drafts after failed creation.
- Add Cmd/Ctrl+Enter submission, initial description focus, and confirmation
  before discarding edited drafts. Escape closes expanded options first;
  backdrop clicks preserve the draft.
- Support light, dark, and Omarchy themes and English, Spanish, French, and German.

### Distribution

- Publish the signed macOS installer and updater for 1.0.12.
- Quit Sonata before replacing the app manually; saved data stays in place.

## 1.0.11 — 2026-11-27

Signed and notarized Apple Silicon macOS DMG and signed automatic update archive.
The Omarchy package remains 1.0.7; AppImage and DEB packages are still pending.

### Fixes

- Fix the crash when creating a task that starts immediately, starting a held
  task, or adding a subtask to active work. Run these desktop commands on the
  application's async runtime so starting background work cannot panic on the
  UI thread.
- Add desktop command regression coverage for bot and workflow execution,
  attachments, manual starts, subtasks, pause/resume, validation errors, and
  recovery of a task saved immediately before the crash.
- Existing saved tasks and queued messages use normal startup recovery; no
  database migration or task recreation is required.

### Distribution

- Publish the signed macOS installer and updater for 1.0.11.
- Quit Sonata before replacing the app manually; saved data stays in place.

## 1.0.10 — 2026-11-27

Signed and notarized Apple Silicon macOS DMG and signed automatic update archive.
The Omarchy package remains 1.0.7; AppImage and DEB packages are still pending.

### Tasks

- Use the new task form everywhere: the board, bot shortcuts, workflow Run,
  newly created bots, and subtasks. Remove the old board and creation screens.
- Keep task creation available while the board loads. Show loading errors with
  Retry, keep existing cards after refresh failures, and preserve an open draft.
- Attach files with the picker or drag-and-drop. Store copies when creating the
  task so bots, workflows, and subtasks can use them after a restart or a wait
  for manual start or dependencies, even if the original files are removed.
- Keep optional titles and required descriptions. Workflow creation uses the
  shared form without the previous workflow preview.
- Preserve existing conversations, scheduled tasks, and archive/restore.

### Distribution

- Publish the signed macOS installer and update archive for 1.0.10. Existing
  1.0.8 and 1.0.9 installations can offer it through their startup update check.
- Quit Sonata before replacing the app manually; saved data stays in place.

## 1.0.9 — 2026-11-27

Signed and notarized Apple Silicon macOS DMG on the homepage, with a signed
application archive for automatic updates. The Omarchy package remains 1.0.7;
AppImage and DEB packages are still pending.

### Improvements

- Make task and subtask titles optional. When left blank, use the first nonempty
  line of the description, shortened to 60 characters. Preserve custom titles
  and the full description; continue requiring a nonblank description.
- Explain optional titles in English, Spanish, French, and German.
- Give Finished task sections a green background that is distinct from Running,
  including dark mode and Omarchy themes.

### Distribution

- Publish the signed, notarized macOS installer and signed update archive for
  1.0.9. Sonata 1.0.8 can offer this release through its startup update check.
- Quit Sonata before replacing the app manually. Saved settings, projects, bots,
  skills, and history stay in place.

## 1.0.8 — 2026-11-27

Signed and notarized Apple Silicon macOS DMG on the homepage. The Omarchy
package remains 1.0.7; AppImage and DEB packages are still pending.

### Tasks and subtasks

- Give every task a stable identity and its own bot or workflow. Users and bots
  can create nested subtasks with the same statuses and conversation controls.
- Add a Pending column, hidden by default, for work waiting to start. Show its
  count, sibling dependencies, and optional manual start controls.
- Notify the immediate parent when a child finishes. Wait for child results and
  reviews before advancing the parent's workflow, with optional automatic
  acceptance of successful results on individual workflow steps.
- Show direct-child progress on parent cards, including Pending and Done tasks,
  and distinguish failed, blocked, and cancelled outcomes.
- Require an explicit final result. Preserve early results across restarts,
  continue independent work after failures, and cancel unfinished descendants
  when their parent is cancelled.
- Keep Finished separate from Done. Archive and restore task trees without
  rerunning work; follow-ups reopen ancestors without rerunning completed siblings.
- Back up the database before migrating existing conversations and orchestration
  history, preserving session IDs and provider context.

### Updates

- Add an **Automatic updates** setting, enabled by default, which checks once at
  startup and offers newer stable versions. Downloading and restarting each
  require confirmation.
- Install verified updates in the app on macOS, AppImage, and DEB installations;
  guide Omarchy and terminal-installer users through the matching update process.
- Publish independently versioned update feeds with signed artifacts, verified
  before deployment. Stop bot work before installing and preserve saved data.

### Distribution

- Publish the signed, notarized 1.0.8 macOS installer and a signed application
  archive for the macOS update feed. Install this first updater-enabled release
  manually; later releases can be offered at startup.
- Quit Sonata before replacing the app. Saved settings, projects, bots, skills,
  and history stay in place.

## 1.0.7 — 2026-11-25

Signed and notarized Apple Silicon macOS DMG on the homepage, plus the Omarchy
package for x86_64. AppImage and DEB packages are still pending.

### Features

- Let Master coordinate linked board tasks with independent bots or different workflows,
  parallel execution, dependencies, and one combined final report.
- Require Master's review before dependent tasks start; support follow-up tasks,
  partial results, cancellation, and recovery after restarting Sonata.
- Show process progress, task results, prerequisites, and restorable completed tasks
  on the board in English, Spanish, French, and German.

### Distribution

- Signed and notarized Apple Silicon macOS DMG for 1.0.7, including Master
  orchestration and the guided onboarding introduced in 1.0.6. Quit Sonata before
  replacing the app; saved data and settings stay in place.
- Arch package `sonata-x86_64.pkg.tar.zst` on the Omarchy GitHub release.
  Install or upgrade it with `pacman -U`. Saved settings, bots, skills and history
  stay in place. Quit Sonata before upgrading.

## 1.0.6 — 2026-11-24

Omarchy package for x86_64. The signed Apple Silicon download on the homepage
remains 1.0.5. AppImage and DEB packages are still pending.

### Features

- Add optional first-run onboarding with job and frequent-task questions, AI-generated
  workflow cards, and review of each workflow's bots and skills before creation.
- Save onboarding progress and support retrying interrupted creation without duplicates.
  Existing installations can open the same assistant from Settings → Personalize workflows.
- Translate onboarding into English, Spanish, French, and German.

### Distribution

- Arch package `sonata-x86_64.pkg.tar.zst` on the Omarchy GitHub release.
  Install or upgrade it with `pacman -U`. Saved settings, bots, skills and history
  stay in place. Quit Sonata before upgrading.

## 1.0.5 — 2026-11-24

Signed and notarized Apple Silicon macOS DMG on the homepage, plus the Omarchy
package for x86_64. AppImage and DEB packages are still pending.

- Keep Grok sessions open while background commands, monitors, loops or subagents
  are running, even when herdr reports the prompt as idle. Restore normal
  30-minute idle cleanup after the background work finishes.
- Show Grok background work on its card and prevent project archiving from
  interrupting it.

### Distribution

- Signed and notarized Apple Silicon macOS DMG for 1.0.5. Download it from the
  homepage; quit Sonata before replacing the app. Saved data and settings stay in place.
- Arch package `sonata-x86_64.pkg.tar.zst` on the Omarchy GitHub release.
  Install or upgrade it with `pacman -U`. Saved settings, bots, skills and history
  stay in place. Quit Sonata before upgrading. This package also includes the
  herdr 0.9.5 runtime from 1.0.4.

## 1.0.4 — 2026-11-24

Signed and notarized Apple Silicon macOS DMG on the homepage. The Omarchy
package remains 1.0.3; AppImage and DEB packages are still pending.

- Upgrade bundled herdr to 0.9.5 for lower headless-pane memory use, more efficient
  terminal reads, improved agent-state detection, and server reliability fixes.
- Cache development runtimes by version so upgrades download the new pinned binary
  without conflicting with an older cached copy.
- Update the About page attribution to Jorge Alvarez and https://alvareznavarro.es.

## 1.0.3 — 2026-11-23

Signed and notarized Apple Silicon macOS DMG on the homepage, plus the Omarchy
package for x86_64. AppImage and DEB packages are still pending.

### Features

- Add projects with independent teams, boards, source directories, schedules, and workflows.
- Inherit the project’s default AI provider when creating bots through the UI or `sbot`.
- Add project selection, editing, archiving, and restoration while other projects continue working.
- Migrate existing installations to **Default**, preserving history and workspace paths, with a database backup.
- Scope configuration sharing to the selected project; keep skills shared and existing YAML files compatible.

### Fixes

- Archiving one project no longer pauses bots in the others while Sonata reads their terminals.
- Creating a bot no longer uses a different folder when the project's workspace cannot be read.

### Distribution

- Signed and notarized Apple Silicon macOS DMG for 1.0.3, including configuration
  sharing, completed-task restoration, and project support. Download it from the
  homepage; quit Sonata before replacing the app. Saved data and settings stay in place.
- Arch package `sonata-x86_64.pkg.tar.zst` on the Omarchy GitHub release.
  Install or upgrade it with `pacman -U`. Saved settings, bots, skills and history
  stay in place. Quit Sonata before upgrading.

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
