#!/bin/bash
# Sonata desktop installer. Requires only system tools; compatible with macOS Bash 3.2.
set -euo pipefail

fail() { printf 'Sonata: %s\n' "$*" >&2; exit 1; }
log() { printf 'Sonata: %s\n' "$*"; }
usage() {
  cat <<'EOF'
Install or update Sonata for Apple Silicon macOS or x86_64/ARM64 Linux.
Usage: install.sh [--version VERSION] [--dest PATH]
  --version VERSION  A stable version such as 1.0.0 (default: latest)
  --dest PATH        Mac: /Applications/Sonata.app
                     Linux: ~/.local/opt/sonata
  -h, --help         Show this help
Finish your work and quit Sonata before updating. Saved data is preserved.
EOF
}

download() {
  case "$1" in
    https://*) curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL --retry 3 --connect-timeout 15 --max-time 300 "$1" -o "$2" ;;
    file://*) cp "${1#file://}" "$2" ;; # Offline release fixtures, used by installer tests.
    *) fail "Release URLs must use HTTPS." ;;
  esac
}

checksum() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
  else shasum -a 256 "$1" | awk '{print $1}'; fi
}

running_check() {
  local processes
  processes="$(ps -axo uid=,comm=)" || fail "Cannot check whether Sonata is running."
  if printf '%s\n' "$processes" | awk -v uid="$(id -u)" '$1 == uid { sub(/^[[:space:]]*[0-9]+[[:space:]]+/, ""); n=split($0,a,"/"); if (a[n] == "sonata") found=1 } END {exit !found}'; then
    fail "Sonata is running. Finish your work, choose Quit, then run the installer again."
  fi
}

cleanup() {
  local status=$? i
  trap - EXIT HUP INT TERM
  set +e
  if [ "$committed" = 0 ]; then
    for ((i=${#changed[@]}-1; i>=0; i--)); do
      rm -f -- "${changed[i]}"
      if [ -f "$stage/metadata-$i" ]; then mv "$stage/metadata-$i" "${changed[i]}"; fi
    done
    if [ "$swapped" = 1 ]; then rm -rf -- "$dest"; fi
    if [ -n "$stage" ] && [ -d "$stage/previous" ]; then
      if ! mv "$stage/previous" "$dest"; then
        printf 'Sonata: recovery needed; previous installation is at %s/previous\n' "$stage" >&2
        stage="" # Never delete the only recoverable copy.
      fi
    fi
  fi
  if [ -n "$mount" ]; then hdiutil detach "$mount" -quiet || true; fi
  [ -z "$stage" ] || rm -rf -- "$stage"
  [ -z "$temp" ] || rm -rf -- "$temp"
  exit "$status"
}

replace_metadata() {
  local source=$1 target=$2 index=${#changed[@]} next
  [ ! -L "$target" ] && [ ! -d "$target" ] || fail "Refusing to overwrite $target."
  mkdir -p "$(dirname "$target")"
  next="$(mktemp "$(dirname "$target")/.sonata.XXXXXX")"
  cp -p "$source" "$next" || { rm -f "$next"; return 1; }
  if [ -f "$target" ]; then cp -p "$target" "$stage/metadata-$index"; fi
  changed[index]="$target"
  mv -f "$next" "$target"
}

main() {
  local version="" os arch asset base release expected actual current parent candidate desktop_exec installed
  local install_prefix="${SONATA_INSTALL_PREFIX:-$HOME/.local}"
  dest="" temp="" stage="" mount="" swapped=0 committed=0
  changed=()
  while [ $# -gt 0 ]; do
    case "$1" in
      --version|--dest)
        [ $# -ge 2 ] && [ -n "$2" ] || fail "$1 needs a value."
        if [ "$1" = --version ]; then version="${2#v}"; else dest="$2"; fi
        shift 2 ;;
      -h|--help) usage; return ;;
      *) fail "Unknown option: $1. See --help." ;;
    esac
  done
  os="$(uname -s)" arch="$(uname -m)"
  case "$os/$arch" in
    Darwin/arm64) os=macos; arch=aarch64 ;;
    Linux/x86_64) os=linux ;;
    Linux/aarch64|Linux/arm64) os=linux; arch=aarch64 ;;
    *) fail "Supported systems: Apple Silicon macOS and x86_64/ARM64 Linux." ;;
  esac
  if [ "$os" = macos ]; then
    [ "$(sw_vers -productVersion | cut -d . -f 1)" -ge 11 ] || fail "macOS 11 or later is required."
    dest="${dest:-/Applications/Sonata.app}"
    asset=Sonata-macos-aarch64.dmg
    case "$dest" in *.app) ;; *) fail "The Mac destination must end in .app." ;; esac
  else
    command -v xdg-open >/dev/null || fail "Install xdg-utils using your distribution's package manager, then retry."
    if command -v dpkg-query >/dev/null; then
      for installed in sonata sonata-app; do
        if dpkg-query -W -f='${Status}' "$installed" 2>/dev/null | grep -q 'install ok installed'; then
          fail "A DEB-managed Sonata is installed. Update it using the DEB download: https://github.com/jorgegorka/sonata/releases/latest"
        fi
      done
    fi
    dest="${dest:-$install_prefix/opt/sonata}"
    asset="Sonata-linux-$arch.AppImage"
  fi
  case "$dest" in /*) ;; *) fail "--dest must be an absolute path." ;; esac
  case "$dest" in *$'\n'*|*$'\r'*) fail "The destination must not contain line breaks." ;; esac
  [ ! -L "$dest" ] || fail "The destination is a symbolic link; choose a real installation directory."
  running_check
  base="${SONATA_RELEASE_BASE_URL:-https://github.com/jorgegorka/sonata/releases}"
  base="${base%/}"
  temp="$(mktemp -d)"
  trap cleanup EXIT
  trap 'exit 130' HUP INT TERM
  if [ -z "$version" ]; then
    download "$base/latest/download/version.txt" "$temp/version.txt" || fail "No stable release is available, or the release server could not be reached."
    version="$(cat "$temp/version.txt")"
  fi
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "Invalid stable release version: $version"
  current=""
  if [ -e "$dest" ]; then
    [ -d "$dest" ] || fail "The destination is not an application directory."
    if [ "$os" = macos ]; then
      [ "$(/usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' "$dest/Contents/Info.plist" 2>/dev/null)" = io.sonata.desktop ] || fail "The destination is not Sonata."
      current="$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$dest/Contents/Info.plist")"
    else
      [ -f "$dest/.sonata-version" ] || fail "The existing directory is not managed by the Sonata installer."
      current="$(cat "$dest/.sonata-version")"
    fi
    [[ "$current" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "Cannot determine the installed version."
    if [ "$current" = "$version" ]; then log "$version is already installed at $dest."; committed=1; return; fi
    if awk -v old="$current" -v new="$version" 'BEGIN { split(old,a,"."); split(new,b,"."); for(i=1;i<=3;i++) {if(a[i]+0>b[i]+0)exit 0; if(a[i]+0<b[i]+0)exit 1} exit 1 }'; then
      fail "Downgrades are not supported ($current -> $version)."
    fi
  fi
  parent="$(dirname "$dest")"
  if ! mkdir -p "$parent" || [ ! -w "$parent" ]; then
    fail "Cannot write to $parent. On Mac, retry with --dest \"\$HOME/Applications/Sonata.app\"."
  fi
  release="$base/download/v$version"
  log "Downloading Sonata $version for $os/$arch..."
  download "$release/SHA256SUMS" "$temp/SHA256SUMS"
  expected="$(awk -v name="$asset" '$2 == name {print $1}' "$temp/SHA256SUMS")"
  [ -n "$expected" ] || fail "Sonata $version has no package for $os/$arch yet. See https://github.com/jorgegorka/sonata/releases for available downloads."
  [[ "$expected" =~ ^[0-9a-f]{64}$ ]] || fail "The release has no valid checksum for $asset."
  download "$release/$asset" "$temp/$asset"
  actual="$(checksum "$temp/$asset")"
  [ "$actual" = "$expected" ] || fail "Download checksum mismatch; the existing installation is unchanged."
  stage="$(mktemp -d "$parent/.sonata-install.XXXXXX")"
  candidate="$stage/next"
  if [ "$os" = macos ]; then
    mount="$temp/mount"
    mkdir "$mount"
    hdiutil attach "$temp/$asset" -readonly -nobrowse -noautoopen -mountpoint "$mount" -quiet
    [ "$(/usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' "$mount/Sonata.app/Contents/Info.plist")" = io.sonata.desktop ] || fail "The download is not Sonata."
    [ "$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$mount/Sonata.app/Contents/Info.plist")" = "$version" ] || fail "The downloaded app has the wrong version."
    codesign --verify --deep --strict "$mount/Sonata.app"
    spctl --assess --type execute "$mount/Sonata.app"
    ditto "$mount/Sonata.app" "$candidate"
    codesign --verify --deep --strict "$candidate"
  else
    chmod 755 "$temp/$asset"
    (cd "$stage" && "$temp/$asset" --appimage-extract >/dev/null)
    mv "$stage/squashfs-root" "$candidate"
    [ -x "$candidate/AppRun" ] && [ -x "$candidate/usr/bin/sonata" ] || fail "Incomplete application bundle."
    printf '%s\n' "$version" > "$candidate/.sonata-version"
    local bin="$install_prefix/bin/sonata" xdg="${XDG_DATA_HOME:-$HOME/.local/share}"
    if [ -e "$bin" ] && ! grep -q '^# Sonata installer$' "$bin"; then fail "Another command owns $bin."; fi
    printf '#!/bin/bash\n# Sonata installer\nexec %q "$@"\n' "$dest/AppRun" > "$stage/launcher"
    chmod 755 "$stage/launcher"
    desktop_exec="$(printf '%s' "$bin" | sed 's/[\\"`$]/\\&/g')"
    printf '[Desktop Entry]\nType=Application\nName=Sonata\nComment=Coordinate your AI coding agents\nExec="%s"\nIcon=io.sonata.desktop\nTerminal=false\nCategories=Development;\nStartupWMClass=Sonata\n' "$desktop_exec" > "$stage/desktop"
    chmod 644 "$stage/desktop"
    local icon="" icon_size
    for icon_size in 512x512 256x256 128x128 32x32; do
      icon="$candidate/usr/share/icons/hicolor/$icon_size/apps/sonata.png"
      [ ! -f "$icon" ] || break
    done
    [ -f "$icon" ] || fail "The bundle has no application icon."
    cp "$icon" "$stage/icon.png"
    chmod 644 "$stage/icon.png"
  fi
  running_check
  if [ -d "$dest" ]; then mv "$dest" "$stage/previous"; fi
  mv "$candidate" "$dest"
  swapped=1
  if [ "$os" = linux ]; then
    replace_metadata "$stage/launcher" "$bin"
    replace_metadata "$stage/desktop" "$xdg/applications/io.sonata.desktop.desktop"
    replace_metadata "$stage/icon.png" "$xdg/icons/hicolor/$icon_size/apps/io.sonata.desktop.png"
    if command -v update-desktop-database >/dev/null; then update-desktop-database "$xdg/applications" >/dev/null 2>&1 || true; fi
  fi
  committed=1
  log "Installed $version at $dest. Open Sonata from your applications menu."
  if [ "$os" = linux ]; then
    case ":$PATH:" in *":$install_prefix/bin:"*) ;; *) log "Add $install_prefix/bin to PATH to use the sonata command." ;; esac
  fi
}

main "$@"
