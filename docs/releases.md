# Releasing Sonata

Stable releases support Apple Silicon macOS 11+ and x86_64/ARM64 Linux desktops.
Linux packages are built on Ubuntu 22.04; the supported baseline is Ubuntu 22.04,
Debian 12, or a compatible system with WebKitGTK 4.1. There is no headless edition.

## Release hosting

### Repository-hosted 1.0.5 download

The notarized Apple Silicon DMG is tracked as `site/downloads/Sonata_1.0.5.dmg`
and served by GitHub Pages. Its `SHA256SUMS` and `version.txt` are stored beside it.
When the latest GitHub Release has no desktop packages, Pages verifies that checksum and renders the local
download link. The terminal installer targets GitHub Releases and is not advertised
for this repository-hosted download.

The previous 1.0.0, 1.0.3 and 1.0.4 DMGs remain available at their original URLs. The secret scanner
allows only these exact DMG paths and digests; other archives and private files
remain excluded. A new repository-hosted installer requires updating
that pin after signing, notarization and verification. Prefer GitHub Release assets
for future versions when an account with release write permission is available.

### Automated GitHub Releases

Enable GitHub Pages with **GitHub Actions** as its source. Add these repository or
environment secrets for the Mac signing job:

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID Application certificate exported as `.p12`, including its private key |
| `APPLE_CERTIFICATE_PASSWORD` | Password of the exported `.p12` |
| `APPLE_SIGNING_IDENTITY` | Full `Developer ID Application: …` identity |
| `APPLE_API_KEY` | App Store Connect API key ID |
| `APPLE_API_ISSUER` | API issuer ID |
| `APPLE_API_PRIVATE_KEY` | Contents of the corresponding `.p8` key |

Create the API key in App Store Connect under **Users and Access → Integrations →
App Store Connect API → Team Keys**, with **Developer** access. Download its `.p8`
file once and save the Key ID and Issuer ID. The Developer ID Application certificate
and team API key can be reused for other apps from the same Apple Developer team.
See [Apple's certificate guide](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/)
and [API key guide](https://developer.apple.com/help/app-store-connect/get-started/app-store-connect-api/).

The signing script creates a
temporary keychain, restores the original search list, and removes its temporary
credentials on exit. Never commit certificates or keys. Missing credentials,
invalid signatures or failed notarization stop publication. Developer unsigned
builds are never used as a fallback for public Mac releases.

## Build and notarize locally on Mac

Install your Developer ID Application certificate and its private key in your login
keychain. Set `APPLE_SIGNING_IDENTITY` to its full identity, as listed by
`security find-identity -v -p codesigning`. Configure one notarization method in
your local environment:

- **Apple Account:** `APPLE_ID` (email), `APPLE_PASSWORD` (app-specific password),
  and `APPLE_TEAM_ID` (the same team that issued the certificate).
- **App Store Connect API:** `APPLE_API_KEY` (Key ID), `APPLE_API_ISSUER` (Issuer ID),
  and `APPLE_API_KEY_PATH` (absolute path to the downloaded `.p8` file).

The local API method uses a file path; CI's `APPLE_API_PRIVATE_KEY` secret contains
the key itself and `scripts/macos-release.sh` writes it to a temporary file.

From the repository root:

```sh
python3 scripts/release.py check-version
npm --prefix app run bundle
codesign --verify --deep --strict target/release/bundle/macos/Sonata.app
xcrun stapler validate target/release/bundle/macos/Sonata.app
spctl --assess --type execute target/release/bundle/macos/Sonata.app
npm --prefix app run dmg
xcrun stapler validate target/release/bundle/dmg/Sonata_1.0.5.dmg
```

Replace the DMG version for later releases. A signed artifact is ready to distribute
only after Apple accepts its notarization and the ticket validates. These commands
create local artifacts; publication uses the workflow below.

## Prepare and publish

1. Update the version together in workspace `Cargo.toml`, `app/package.json`,
   `app/package-lock.json` (including the root package), and
   `app/src-tauri/tauri.conf.json`. Update the workspace packages in `Cargo.lock`,
   add the release to `CHANGELOG.md`, and update the README version and examples.
2. Run `python3 scripts/release.py check-version`, the desktop checks, and the
   acceptance checks below. Review the final commit before tagging it.
3. Push `v<version>` only when ready to publish. The tag must match the source
   version, and an existing release cannot be overwritten. Tags trigger the
   release workflow; ordinary branch pushes only run checks.

The workflow tests and builds the Mac DMG plus both Linux architectures. It
collects checksums from the individual builds and validates the complete matrix
before creating a draft release. Only after uploading every asset does it publish
the release and request a Pages deployment. If uploading fails, the draft is not
advertised; inspect and remove an incomplete draft before retrying the workflow.

Assets have stable names within immutable version tags:

- `Sonata-macos-aarch64.dmg`
- `Sonata-linux-x86_64.AppImage` and `Sonata-linux-x86_64.deb`
- `Sonata-linux-aarch64.AppImage` and `Sonata-linux-aarch64.deb`
- `SHA256SUMS` and `version.txt`

An explicitly selected platform can also be published independently after its
checks pass. For example, 1.0.0 initially ships the notarized macOS DMG:

```sh
python3 scripts/release.py collect --platform macos-aarch64
python3 scripts/release.py finalize --platform macos-aarch64
```

Upload only that platform's packages, `SHA256SUMS`, and `version.txt` to a draft
GitHub release targeting the matching source commit, verify the assets, then publish.
Keep the release notes and README explicit about which platforms are available.
The automated tag workflow still requires the complete platform matrix. Published
assets are immutable; ship additional platforms in a new release.

The installer resolves `version.txt` once, then fetches artifacts and checksums
from that version's tag. Pages resolves a stable release during deployment and
writes direct links only for complete platform packages. Unavailable packages are
marked as coming soon, and the general terminal installer panel appears once all
platforms are available. There are no GitHub API calls
from visitors' browsers, and downloads work with JavaScript disabled. Before the
first release, the source page explicitly says that packaged downloads are not
available. Missing checksums or an incomplete platform (such as a Linux AppImage
without its DEB) fail deployment, as does a failed metadata request, retaining the
previous site.

## Omarchy package

Omarchy installs an Arch package named `sonata-$(uname -m).pkg.tar.zst`. Publish
that file on a GitHub release whose tag does not match `v<version>`. A `v*` tag
starts the desktop workflow above, which expects the Mac DMG and the Linux
AppImage and DEB packages. The 1.0.3 Omarchy release uses the tag `omarchy-1.0.3`
and ships `sonata-x86_64.pkg.tar.zst`.

The homepage reads the latest GitHub release when Pages deploys. A release with
no desktop packages leaves the repository-hosted 1.0.5 DMG in place. A desktop
release that is missing part of a platform still fails that deploy.

Build the package from a checkout with `pkg/PKGBUILD` (`SONATA_SRC` must point
at the source tree), then upload the archive under the stable asset name:

```sh
cp sonata-1.0.3-1-x86_64.pkg.tar.zst sonata-x86_64.pkg.tar.zst
gh release create omarchy-1.0.3 sonata-x86_64.pkg.tar.zst \
  --target master --title "Sonata 1.0.3" --notes-file /tmp/omarchy-notes.md
```

Quit Sonata before upgrading an existing install with `sudo pacman -U`.

## Validation

```sh
cargo test --locked --workspace
npm --prefix app ci --engine-strict
npm --prefix app test
npm --prefix app run build
python3 -m unittest discover -s scripts/tests -v
```

To reproduce Linux builds in a disposable container:

```sh
docker build -t sonata-linux -f scripts/linux.Dockerfile scripts
mkdir -p release-dist
docker run --rm -v "$PWD:/source:ro" -v "$PWD/release-dist:/artifacts" sonata-linux --package
```

This runs both test suites and the real herdr socket tests without paid agent CLIs.
Native WebKit automation checks setup and runtime health in the actual packages,
including an AppImage installed through the terminal installer, and a DEB installed
by `dpkg` inside the disposable container. The AppImage also exercises native picker
selection and comment submission, and verifies that page scripts and iframes cannot
access its bridge. The source mount is read-only. Compilation and tests run as an
unprivileged user in a disposable copy; only packaged artifacts are copied to
`release-dist`.

Before publishing a first release or changing native integration, also check:

- A browser-downloaded DMG opens normally and its installed app starts on a clean
  supported Mac. `codesign`, `spctl` and `stapler` checks pass in the signing job.
- Both Linux packages reach setup and find their private `herdr` and `sbot`,
  including launch from the desktop menu with a limited shell PATH.
- X11 and Wayland: restore a minimized window, use menus, notifications, file
  opening, trash, language selection and Ctrl shortcuts.
- In the native embedded browser, select an element, scroll it, type and submit a
  comment, cancel, navigate and return. Page scripts, iframes, oversized messages
  and messages from the previous page must not bypass the picker validation.
- On each platform, install a new version over an older installation and confirm
  saved settings, bots, skills and history survive. Finish work and quit first;
  this release does not preserve running bot processes across upgrades.
- Verify the homepage at mobile/desktop sizes, with keyboard navigation,
  clipboard failure and JavaScript disabled.

Installer tests use a local `file://` fixture through `SONATA_RELEASE_BASE_URL`;
ordinary installations always use the project's HTTPS GitHub release URLs. The
tests never alter the caller's home directory or an installed Sonata.

## Runtime changes

`scripts/herdr-runtime.sh` is the source of truth for herdr's pinned version and
per-target checksums. Update every supported target together and run the real
herdr-client tests on each platform. Sidecar preparation respects the build target;
Linux runtime helpers are resources, while Mac helpers are signed app sidecars.
Changing installation code does not migrate application databases.

Downloaded runtimes are cached in `.tools/herdr/<version>/<target>/herdr`.
Setup keeps `.tools/herdr/herdr` pointing to the selected runtime; old caches can
coexist with a new version. An explicit `SONATA_HERDR_BIN` must match the selected
target's pinned checksum and is never overwritten by the downloader.
Mac and Linux checks run the Rust workspace with the verified runtime so live
socket and runtime lifecycle tests execute. Paid provider tests remain opt-in.

After changing Grok detection, check its live background-work lifecycle with a
signed-in Grok CLI (this starts a disposable session and uses provider credits):

```sh
SONATA_HERDR_BIN="$PWD/.tools/herdr/herdr" cargo test --locked -p sonata-core \
  --test herdr_live grok_idle_background_work_is_detected_until_the_command_finishes \
  -- --ignored --nocapture
```

This checks that background work remains detectable when herdr reports Grok idle,
then clears after the command finishes. The watchdog regression test advances a
paused clock across repeated 30-minute cleanup checks and verifies eventual cleanup.
