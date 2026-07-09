#!/usr/bin/env bash
# ============================================================
# Catalyst Code — Installer  v1.1.0
# Platform: Linux (systemd) & macOS (launchd)
#
# DEFAULT: download prebuilt binaries (catcode + catcode-core) and, with
# --with-web, a prebuilt Next.js web bundle — NO compiler (cargo/go/next
# build) is needed on the host. The TUI needs zero host deps; the web
# service only needs a Node OR Bun runtime to run (not to build).
#
#   bash install.sh                       # download + install catcode
#   bash install.sh --with-web             # …also install the web service
#   bash install.sh --version 0.2.0        # pin a version
#   bash install.sh --base-url <url>       # download from a mirror (not GitHub)
#   bash install.sh --build-from-source    # fall back to cargo+go+next build
#   bash install.sh --update               # re-download latest + reinstall
#   bash install.sh --uninstall            # remove everything
#
# Options:
#   --with-web            install the web frontend service
#   --version <v>         pin a release version (e.g. 0.2.0 or v0.2.0)
#   --base-url <url>      download base URL (default: GitHub Releases)
#   --build-from-source   build locally instead of downloading prebuilt
#   --repo <url>          (source path) clone <url> first
#   --prefix <path>       binary install dir   (default: /usr/local/bin)
#   --web-dir <path>      web bundle install dir (default: /opt/catalyst-code/web
#                         on Linux, ~/Library/Application Support/catalyst-code/web
#                         on macOS)
#   --port <n>            web service port      (default: 49283)
#   --host <h>            web bind host         (default: 0.0.0.0)
#   --log-file <path>     write a log here      (default: ~/catalyst-code-install.log)
#   --no-log              disable logging
#   --no-color           disable ANSI colors
#   --dry-run            print the plan, execute nothing
#   -h, --help           show this help
# ============================================================
set -euo pipefail

# ── constants ────────────────────────────────────────────────
APP_NAME="Catalyst Code"
VERSION="1.1.0"
GITHUB_REPO="catalystctl/catcode"
DEFAULT_PREFIX="/usr/local/bin"
DEFAULT_PORT="49283"
DEFAULT_HOST="0.0.0.0"
STATE_FILE="/etc/catalyst-code/installer.state"
UNIT_NAME="catalyst-code-web.service"   # systemd unit (Linux)
LAUNCHD_LABEL="com.catalyst-code.web"   # launchd agent (macOS)
GO_MIN="1.25.0"

# ── platform ────────────────────────────────────────────────
PLATFORM="$(uname -s)"   # Linux | Darwin
case "$PLATFORM" in
  Linux)  SVC_MGR="systemd" ;;
  Darwin) SVC_MGR="launchd" ;;
  *)      SVC_MGR="unsupported" ;;
esac

# ── option defaults ──────────────────────────────────────────
ACTION="install"
DRY_RUN=false
WITH_WEB=false
BUILD_FROM_SOURCE=false
VERSION_OVERRIDE=""
BASE_URL_OVERRIDE=""
WEB_DIR_OVERRIDE=""
REPO_OVERRIDE=""
PREFIX="$DEFAULT_PREFIX"
PORT="$DEFAULT_PORT"
HOST="$DEFAULT_HOST"
LOG_FILE="${HOME}/catalyst-code-install.log"
NO_COLOR_FLAG=false
LOG_ENABLED=false

# ── runtime state ────────────────────────────────────────────
SUDO=""
RUNTIME=""        # bun | npm | node
RT_BIN=""         # absolute path to bun/node
RT=""             # runtime word for "run"/"install" invocations
REPO_DIR=""
ORIGIN_URL=""
INSTALL_USER=""
VERSION_DETECTED="$VERSION"
# download-path state
OS_TAG=""
ARCH=""
TAG=""            # e.g. v0.2.0
VER=""            # e.g. 0.2.0
BASE_URL=""
WEB_DIR=""
METHOD="download" # download | source

# ── temp dir + cleanup ───────────────────────────────────────
TMPDIR_SELF=""
cleanup() {
  [[ -n "${TMPDIR_SELF:-}" && -d "${TMPDIR_SELF:-}" ]] && rm -rf "$TMPDIR_SELF"
}
trap cleanup EXIT
TMPDIR_SELF="$(mktemp -d -t catalyst-inst.XXXXXX 2>/dev/null || mktemp -d)"

# ── colors (degrade gracefully) ──────────────────────────────
USE_COLOR=true
[[ -t 1 ]] || USE_COLOR=false
[[ -z "${NO_COLOR:-}" ]] || USE_COLOR=false
$NO_COLOR_FLAG && USE_COLOR=false
if $USE_COLOR; then
  C_RED=$'\e[31m'; C_GREEN=$'\e[32m'; C_YELLOW=$'\e[33m'
  C_CYAN=$'\e[36m'; C_DIM=$'\e[2m'; C_BOLD=$'\e[1m'; C_RST=$'\e[0m'
else
  C_RED=""; C_GREEN=""; C_YELLOW=""; C_CYAN=""; C_DIM=""; C_BOLD=""; C_RST=""
fi

# ── helpers ──────────────────────────────────────────────────
have() { command -v "$1" >/dev/null 2>&1; }

_log() { $LOG_ENABLED && printf '%s\n' "$1" >>"$LOG_FILE" 2>/dev/null || true; }

die() {
  printf "\n${C_BOLD}${C_RED}error:${C_RST}${C_RED} %s${C_RST}\n" "$*" >&2
  _log "[FATAL] $*"
  exit 1
}

log_info() { printf "  ${C_CYAN}ℹ${C_RST} %s\n" "$*"; _log "INFO: $*"; }
log_ok()   { printf "  ${C_GREEN}✓${C_RST} %s\n" "$*";  _log "OK:   $*"; }
log_warn() { printf "  ${C_YELLOW}⚠${C_RST} %s\n" "$*" >&2; _log "WARN: $*"; }

phase() {
  printf "\n${C_BOLD}${C_CYAN}▸ %s${C_RST}\n" "$*"
  printf "${C_DIM}────────────────────────────────────────────────${C_RST}\n"
  _log ""
  _log ":: $*"
}

# run_step [--cwd DIR] "message" command [args...]
# Shows a spinner while the command runs; captures output, shows tail on failure.
run_step() {
  local cwd=""
  [[ "${1:-}" == "--cwd" ]] && { cwd="$2"; shift 2; }
  local msg="$1"; shift
  local rc=0 log
  log="$(mktemp -p "$TMPDIR_SELF")" || die "mktemp failed"
  if $DRY_RUN; then
    printf "  ${C_DIM}○${C_RST} [dry-run] %s\n" "$msg"
    if [[ -n "$cwd" ]]; then
      printf "      ${C_DIM}would run (in %s):%s${C_RST}\n" "$cwd" " $*"
    else
      printf "      ${C_DIM}would run:%s${C_RST}\n" " $*"
    fi
    _log "[dry-run] $msg :: $*"
    return 0
  fi
  local spin=('⠋' '⠙' '⠹' '⠸' '⠼' '⠴' '⠦' '⠧' '⠇' '⠏')
  local i=0
  if [[ -n "$cwd" ]]; then
    ( cd "$cwd" && "$@" ) >"$log" 2>&1 &
  else
    "$@" >"$log" 2>&1 &
  fi
  local pid=$!
  while kill -0 "$pid" 2>/dev/null; do
    printf "\r  ${C_CYAN}%s${C_RST} %s" "${spin[$((i % 10))]}" "$msg"
    i=$((i + 1))
    sleep 0.08
  done
  wait "$pid" || rc=$?
  if [[ $rc -eq 0 ]]; then
    printf "\r  ${C_GREEN}✔${C_RST} %s\n" "$msg"
    _log "[ok] $msg"
    $LOG_ENABLED && cat "$log" >>"$LOG_FILE" 2>/dev/null || true
  else
    printf "\r  ${C_RED}✖${C_RST} %s\n" "$msg"
    echo "    ${C_RED}--- last 30 lines of output ---${C_RST}" >&2
    tail -n 30 "$log" >&2 2>/dev/null || true
    _log "[FAIL] $msg"
    $LOG_ENABLED && cat "$log" >>"$LOG_FILE" 2>/dev/null || true
  fi
  return $rc
}

# run_root "message" command [args...] — like run_step but prefixed with sudo when needed.
run_root() {
  local msg="$1"; shift
  if [[ -n "$SUDO" ]]; then run_step "$msg" "$SUDO" "$@"
  else run_step "$msg" "$@"; fi
}

as_root() { if [[ $EUID -eq 0 ]]; then "$@"; else sudo "$@"; fi; }
root_do() {
  local msg="$1"; shift
  if $DRY_RUN; then
    printf "  ${C_DIM}○ [dry-run] %s${C_RST}\n    would: %s\n" "$msg" "$*"
    return 0
  fi
  if as_root "$@" >/dev/null 2>&1; then
    printf "  ${C_GREEN}✓${C_RST} %s\n" "$msg"
  else
    printf "  ${C_YELLOW}⚠${C_RST} %s ${C_DIM}(skipped)${C_RST}\n" "$msg"
  fi
  _log "do: $msg :: $*"
}

# ver_ge REQUIRED ACTUAL → 0 if ACTUAL >= REQUIRED (dot-numeric, up to 3 parts)
ver_ge() {
  awk -v r="$1" -v a="$2" 'BEGIN{
    n=split(r,R,"."); m=split(a,A,".");
    for(i=1;i<=3;i++){ rv=(i<=n)?R[i]+0:0; av=(i<=m)?A[i]+0:0;
      if(av>rv) exit 0; if(av<rv) exit 1; }
    exit 0;
  }'
}

# ── banner / box ─────────────────────────────────────────────
print_box() {
  local title="$1"; shift
  local lines=("$@")
  local W=${#title}
  local l
  for l in "${lines[@]}"; do [[ ${#l} -gt $W ]] && W=${#l}; done
  local bar=""
  local _
  for _ in $(seq 1 $((W + 2))); do bar+="─"; done
  local F="${C_BOLD}${C_CYAN}" R="${C_RST}"
  printf "\n"
  printf "  ${F}┌${bar}┐${R}\n"
  printf "  ${F}│${R} %-${W}s ${F}│${R}\n" "$title"
  if [[ ${#lines[@]} -gt 0 ]]; then
    printf "  ${F}├${bar}┤${R}\n"
    for l in "${lines[@]}"; do printf "  ${F}│${R} %-${W}s ${F}│${R}\n" "$l"; done
  fi
  printf "  ${F}└${bar}┘${R}\n\n"
}

print_banner() {
  local mode="download (prebuilt)"
  $BUILD_FROM_SOURCE && mode="build-from-source"
  print_box "Catalyst Code  —  installer v${VERSION_DETECTED}" \
    "TUI (catcode) + core (catcode-core) -> PATH" \
    "optional 24/7 web service (Next.js, prebuilt)" \
    "scope: system-wide   |   platform: ${PLATFORM} (${SVC_MGR})"
  printf "  ${C_DIM}mode: %s   |   dry-run: %s${C_RST}\n\n" "$mode" "$DRY_RUN"
}

# ── arg parsing ──────────────────────────────────────────────
usage() {
  awk 'NR==1{next} /^#/{print; next} {exit}' "$0" | sed 's/^# \{0,1\}//'
  exit 0
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --install)            ACTION="install" ;;
      --update|--upgrade)   ACTION="update" ;;
      --uninstall)          ACTION="uninstall" ;;
      --dry-run)            DRY_RUN=true ;;
      --with-web)           WITH_WEB=true ;;
      --build-from-source)  BUILD_FROM_SOURCE=true; METHOD="source" ;;
      --version)            [[ $# -ge 2 ]] || die "--version requires a value"; VERSION_OVERRIDE="$2"; shift ;;
      --base-url)           [[ $# -ge 2 ]] || die "--base-url requires a URL"; BASE_URL_OVERRIDE="$2"; shift ;;
      --web-dir)            [[ $# -ge 2 ]] || die "--web-dir requires a path"; WEB_DIR_OVERRIDE="$2"; shift ;;
      --repo)               [[ $# -ge 2 ]] || die "--repo requires a URL"; REPO_OVERRIDE="$2"; shift ;;
      --prefix)             [[ $# -ge 2 ]] || die "--prefix requires a path"; PREFIX="$2"; shift ;;
      --port)               [[ $# -ge 2 ]] || die "--port requires a number"; PORT="$2"; shift ;;
      --host)               [[ $# -ge 2 ]] || die "--host requires a value"; HOST="$2"; shift ;;
      --log-file)           [[ $# -ge 2 ]] || die "--log-file requires a path"; LOG_FILE="$2"; shift ;;
      --no-log)             LOG_FILE="" ;;
      --no-color)           NO_COLOR_FLAG=true ;;
      -h|--help)            usage ;;
      *)                    die "unknown option: $1 (try --help)" ;;
    esac
    shift
  done
}

# ── log file setup ───────────────────────────────────────────
setup_log() {
  [[ -z "${LOG_FILE:-}" ]] && { LOG_ENABLED=false; return; }
  if ! { mkdir -p "$(dirname "$LOG_FILE")" 2>/dev/null && : >"$LOG_FILE" 2>/dev/null; }; then
    log_warn "cannot write log to $LOG_FILE — logging disabled"
    LOG_FILE=""; LOG_ENABLED=false; return
  fi
  LOG_ENABLED=true
  _log "===== catalyst-code install.sh — $(date -u +%FT%TZ) ====="
  _log "action=$ACTION dry_run=$DRY_RUN build_from_source=$BUILD_FROM_SOURCE with_web=$WITH_WEB prefix=$PREFIX port=$PORT host=$HOST"
}

# ── sudo ─────────────────────────────────────────────────────
ensure_sudo() {
  if [[ $EUID -eq 0 ]]; then
    SUDO=""
    INSTALL_USER="${SUDO_USER:-root}"
    return
  fi
  have sudo || die "not root and 'sudo' is unavailable — run as root or install sudo"
  SUDO="sudo"
  INSTALL_USER="${SUDO_USER:-$USER}"
  if ! $DRY_RUN; then
    if sudo -n true 2>/dev/null; then
      : # passwordless sudo — no prompt needed
    else
      log_info "System-wide install — authenticating with sudo..."
      sudo -v || die "sudo authentication failed"
    fi
  fi
}

# ── runtime detection (bun vs node, for RUNNING the web) ─────
detect_runtime() {
  if have bun; then
    RUNTIME="bun"; RT_BIN="$(command -v bun)"; RT="bun"
  elif have node; then
    RUNTIME="node"; RT_BIN="$(command -v node)"; RT="node"
  elif have npm; then
    RUNTIME="npm"; RT_BIN="$(command -v npm)"; RT="npm"
    have node || die "node not found (npm requires it)"
  else
    die "neither bun nor node found — install one to run the web frontend (https://bun.sh or https://nodejs.org)"
  fi
  log_ok "Web runtime: $RUNTIME ($RT_BIN)"
}

# ════════════════════════════════════════════════════════════
# DOWNLOAD PATH (default — no compile)
# ════════════════════════════════════════════════════════════

detect_os_tag() {
  case "$PLATFORM" in
    Linux)  OS_TAG="linux" ;;
    Darwin) OS_TAG="macos" ;;
    *)      die "install.sh supports Linux and macOS only (this is '$PLATFORM'). Windows users: see packaging/windows/install-web.ps1" ;;
  esac
}

detect_arch() {
  local m; m="$(uname -m)"
  case "$m" in
    x86_64|amd64)  ARCH="x86_64" ;;
    aarch64|arm64) ARCH="arm64"  ;;
    *)             die "unsupported arch: $m (expected x86_64 or arm64)" ;;
  esac
}

# Resolve the release TAG/VER and the download BASE_URL.
#   --version <v>  pins a version (accepts "0.2.0" or "v0.2.0")
#   otherwise      query the GitHub API for the latest release tag
#   --base-url <u> overrides the download root (skips the GitHub default)
resolve_release() {
  if [[ -n "$VERSION_OVERRIDE" ]]; then
    # Accept "0.2.0" (-> v0.2.0 semver tag), "v0.2.0" (as-is), or a commit
    # SHA like "9fecd6b" (as-is — SHA tags have no leading v). Only prepend v
    # for bare semver (digits.digits), never for hex SHAs.
    TAG="$VERSION_OVERRIDE"
    if [[ "$TAG" =~ ^[0-9]+\.[0-9]+ ]] && [[ "$TAG" != v* ]]; then
      TAG="v${TAG}"
    fi
    VER="${TAG#v}"
  else
    local api="https://api.github.com/repos/${GITHUB_REPO}/releases/latest"
    if ! TAG="$(curl -fsSL --retry 2 "$api" 2>/dev/null | jq -r '.tag_name // empty' 2>/dev/null)" || [[ -z "${TAG:-}" ]]; then
      die "could not resolve the latest release from $api.
  The repo may be private, or the API is rate-limited. Pass --version <v>
  (e.g. --version 0.2.0 or --version 9fecd6b) or --base-url <url> to a public mirror."
    fi
    VER="${TAG#v}"
  fi
  VERSION_DETECTED="$VER"
  if [[ -n "$BASE_URL_OVERRIDE" ]]; then
    BASE_URL="${BASE_URL_OVERRIDE%/}"
  else
    BASE_URL="https://github.com/${GITHUB_REPO}/releases/download/${TAG}"
  fi
}

# verify_sha256 <file> <shafile>  — compares the recorded hash to the file.
verify_sha256() {
  local f="$1" sf="$2" expected actual
  [[ -f "$sf" ]] || die "missing checksum file: $sf"
  expected="$(awk '{print $1; exit}' "$sf")"
  actual="$(sha256sum "$f" | awk '{print $1}')"
  if [[ "$expected" != "$actual" ]]; then
    die "checksum mismatch for $(basename "$f")
  expected $expected
  got      $actual"
  fi
}

# fetch_asset <name>  — download <BASE_URL>/<name> + <name>.sha256 into the
# temp dir and verify the checksum. Leaves the file at $TMPDIR_SELF/<name>.
fetch_asset() {
  local name="$1"
  local url="${BASE_URL}/${name}"
  local dest="$TMPDIR_SELF/$name"
  run_step "Downloading $name" curl -fL --retry 3 -o "$dest" "$url" \
    || die "download failed: $url"
  run_step "Downloading $name.sha256" curl -fL --retry 3 -o "$dest.sha256" "${url}.sha256" \
    || die "checksum download failed: ${url}.sha256"
  if $DRY_RUN; then return 0; fi
  verify_sha256 "$dest" "$dest.sha256"
  log_ok "Verified $name"
}

resolve_web_dir() {
  if [[ -n "$WEB_DIR_OVERRIDE" ]]; then
    WEB_DIR="$WEB_DIR_OVERRIDE"; return
  fi
  if [[ "$PLATFORM" == "Darwin" ]]; then
    WEB_DIR="$HOME/Library/Application Support/catalyst-code/web"
  else
    WEB_DIR="/opt/catalyst-code/web"
  fi
}

# Download + install the TUI standalone (and, with --with-web, the core binary).
install_bins_download() {
  local tui_asset="catcode-${VER}-${OS_TAG}-${ARCH}"
  fetch_asset "$tui_asset"
  run_root "Creating $PREFIX" mkdir -p "$PREFIX"
  if ! $DRY_RUN; then
    [[ -f "$TMPDIR_SELF/$tui_asset" ]] || die "downloaded TUI binary missing"
  fi
  run_root "Installing catcode -> $PREFIX/catcode" install -m 0755 "$TMPDIR_SELF/$tui_asset" "$PREFIX/catcode"

  if $WITH_WEB; then
    local core_asset="catcode-core-${VER}-${OS_TAG}-${ARCH}"
    fetch_asset "$core_asset"
    run_root "Installing catcode-core -> $PREFIX/catcode-core" install -m 0755 "$TMPDIR_SELF/$core_asset" "$PREFIX/catcode-core"
  fi
}

# Download + extract the prebuilt web bundle and wire the service.
install_web_download() {
  detect_runtime
  local web_asset="catcode-web-${VER}.tar.gz"
  fetch_asset "$web_asset"
  resolve_web_dir
  run_root "Creating $WEB_DIR" mkdir -p "$WEB_DIR"
  # Clean stale contents so an update doesn't leave old chunks.
  if ! $DRY_RUN; then
    find "$WEB_DIR" -mindepth 1 -delete 2>/dev/null || true
  fi
  run_root "Extracting web bundle -> $WEB_DIR" tar xzf "$TMPDIR_SELF/$web_asset" -C "$WEB_DIR"
  if ! $DRY_RUN; then
    [[ -f "$WEB_DIR/start.js" ]] || die "web bundle missing start.js (extraction failed?)"
  fi
  install_web_service_download
}

# systemd unit (Linux) — runs the prebuilt standalone server (node start.js).
install_web_systemd_download() {
  local unit_dir="/etc/systemd/system"
  local unit="$unit_dir/$UNIT_NAME"
  local tmp; tmp="$(mktemp -p "$TMPDIR_SELF")"
  cat >"$tmp" <<EOF
[Unit]
Description=Catalyst Code Web Frontend (port $PORT)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$INSTALL_USER
WorkingDirectory=$WEB_DIR
Environment=NODE_ENV=production
Environment=PORT=$PORT
Environment=HOSTNAME=$HOST
Environment=CATCODE_CORE=$PREFIX/catcode-core
ExecStart=$RT_BIN $WEB_DIR/start.js
Restart=on-failure
RestartSec=3
# NOTE: runs the prebuilt Next.js standalone server. For public exposure, put
# a reverse proxy (caddy/nginx) with TLS in front and bind --host 127.0.0.1.

[Install]
WantedBy=multi-user.target
EOF
  if $DRY_RUN; then
    log_info "[dry-run] would write unit: $unit"
    sed 's/^/      /' "$tmp"
    return 0
  fi
  run_root "Creating $unit_dir" mkdir -p "$unit_dir"
  run_root "Installing unit file" install -m 0644 "$tmp" "$unit" || die "could not install unit"
  run_root "Reloading systemd" systemctl daemon-reload || die "daemon-reload failed"
  run_root "Enabling $UNIT_NAME" systemctl enable "$UNIT_NAME" || die "enable failed"
  run_root "Starting $UNIT_NAME" systemctl start "$UNIT_NAME" || die "start failed — check: journalctl -u $UNIT_NAME -e"
}

# launchd agent (macOS) — runs the prebuilt standalone server (node start.js).
install_web_launchd_download() {
  local agents_dir="$HOME/Library/LaunchAgents"
  local plist="$agents_dir/${LAUNCHD_LABEL}.plist"
  local log_dir="$HOME/Library/Logs"
  local log_file="$log_dir/catalyst-code-web.log"
  local tmp; tmp="$(mktemp -p "$TMPDIR_SELF")"
  cat >"$tmp" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${RT_BIN}</string>
    <string>${WEB_DIR}/start.js</string>
  </array>
  <key>WorkingDirectory</key>
  <string>${WEB_DIR}</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>NODE_ENV</key>
    <string>production</string>
    <key>PORT</key>
    <string>${PORT}</string>
    <key>HOSTNAME</key>
    <string>${HOST}</string>
    <key>CATCODE_CORE</key>
    <string>${PREFIX}/catcode-core</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>${log_file}</string>
  <key>StandardErrorPath</key>
  <string>${log_file}</string>
</dict>
</plist>
EOF
  if $DRY_RUN; then
    log_info "[dry-run] would write plist: $plist"
    sed 's/^/      /' "$tmp"
    return 0
  fi
  run_step "Creating $agents_dir" mkdir -p "$agents_dir"
  run_step "Creating $log_dir" mkdir -p "$log_dir"
  run_step "Installing plist" install -m 0644 "$tmp" "$plist" || die "could not install plist"
  launchctl unload "$plist" >/dev/null 2>&1 || true
  run_step "Loading $LAUNCHD_LABEL" launchctl load "$plist" || die "load failed — check: cat $log_file"
  log_ok "Web agent loaded (starts at login, auto-restarts on crash)"
}

install_web_service_download() {
  if [[ "$PLATFORM" == "Darwin" ]]; then
    install_web_launchd_download
  else
    install_web_systemd_download
  fi
}

restart_web_service_download() {
  if [[ "$PLATFORM" == "Darwin" ]]; then
    local plist="$HOME/Library/LaunchAgents/${LAUNCHD_LABEL}.plist"
    launchctl unload "$plist" >/dev/null 2>&1 || true
    run_step "Reloading $LAUNCHD_LABEL" launchctl load "$plist" || die "load failed — check: cat $HOME/Library/Logs/catalyst-code-web.log"
  else
    run_root "Reloading systemd" systemctl daemon-reload
    run_root "Restarting $UNIT_NAME" systemctl restart "$UNIT_NAME" || die "restart failed — check: journalctl -u $UNIT_NAME -e"
  fi
}

check_deps_download() {
  local missing=()
  have curl     || missing+=("curl")
  have sha256sum || missing+=("sha256sum (coreutils)")
  if $WITH_WEB; then
    have bun || have node || have npm || missing+=("bun or node (to RUN the web frontend — https://bun.sh or https://nodejs.org)")
  fi
  if [[ ${#missing[@]} -gt 0 ]]; then
    printf "\n  ${C_BOLD}${C_RED}Missing dependencies:${C_RST}\n" >&2
    local m
    for m in "${missing[@]}"; do printf "    ${C_RED}• %s${C_RST}\n" "$m" >&2; done
    die "install the dependencies above, then re-run."
  fi
  [[ "$SVC_MGR" != "unsupported" ]] \
    || die "install.sh supports Linux and macOS only (this is '$PLATFORM'). Windows users: see packaging/windows/install-web.ps1"
  log_ok "Dependencies present (curl, sha256sum${WITH_WEB:+, node/bun})"
}

do_install_download() {
  phase "Checking dependencies"
  check_deps_download
  ensure_sudo
  phase "Resolving release"
  detect_os_tag
  detect_arch
  resolve_release

  log_info "Version:  $VER (tag $TAG)"
  log_info "Source:   $BASE_URL"
  log_info "Platform: $OS_TAG/$ARCH"
  log_info "Prefix:   $PREFIX"
  $WITH_WEB && log_info "Web:      $UNIT_NAME on :$PORT ($HOST)"

  phase "Installing catcode (prebuilt — no compile)"
  install_bins_download

  if $WITH_WEB; then
    phase "Installing web service (prebuilt — no compile)"
    install_web_download
  else
    log_info "Skipping web service (pass --with-web to install it)"
  fi

  save_state
  summary_install
}

do_update_download() {
  phase "Resolving latest release"
  detect_os_tag
  detect_arch
  resolve_release
  log_info "Version:  $VER (tag $TAG)"
  log_info "Source:   $BASE_URL"
  ensure_sudo

  phase "Reinstalling catcode (prebuilt)"
  install_bins_download

  if [[ "${WEB_INSTALLED:-no}" == yes ]]; then
    WITH_WEB=true
    phase "Reinstalling web service (prebuilt)"
    install_web_download
    phase "Restarting web service"
    restart_web_service_download
  fi

  save_state
  summary_update
}

# ════════════════════════════════════════════════════════════
# SOURCE PATH (--build-from-source fallback)
# ════════════════════════════════════════════════════════════

check_deps_source() {
  local missing=()
  have cargo || missing+=("cargo (Rust toolchain — https://rustup.rs)")
  have go    || missing+=("go (>= ${GO_MIN} — https://go.dev/dl/)")
  if have go; then
    local gv
    gv="$(go version 2>/dev/null | sed -E 's/.*go([0-9]+\.[0-9]+(\.[0-9]+)?).*/\1/' || echo 0)"
    ver_ge "$GO_MIN" "$gv" || missing+=("go >= ${GO_MIN} (have ${gv})")
  fi
  if $WITH_WEB; then
    have bun || have npm || missing+=("bun or npm (for the web build — https://bun.sh)")
  fi
  if [[ ${#missing[@]} -gt 0 ]]; then
    printf "\n  ${C_BOLD}${C_RED}Missing dependencies:${C_RST}\n" >&2
    local m
    for m in "${missing[@]}"; do printf "    ${C_RED}• %s${C_RST}\n" "$m" >&2; done
    die "install the dependencies above, then re-run (or drop --build-from-source to download prebuilt binaries)."
  fi
  [[ "$SVC_MGR" != "unsupported" ]] \
    || die "install.sh supports Linux and macOS only (this is '$PLATFORM'). Windows users: see packaging/windows/install-web.ps1"
  log_ok "Dependencies present (cargo, go${WITH_WEB:+, node/bun})"
}

find_repo_root() {
  local d
  d="$(cd "$(dirname "$0")" && pwd)"
  while [[ "$d" != "/" ]]; do
    if [[ -f "$d/core/Cargo.toml" && -f "$d/build.sh" ]]; then printf '%s' "$d"; return 0; fi
    d="$(dirname "$d")"
  done
  return 1
}

resolve_repo() {
  if [[ -n "$REPO_OVERRIDE" ]]; then
    REPO_DIR="${CATALYST_CODE_INSTALL_DIR:-$HOME/catalyst-code}"
    if $DRY_RUN; then
      run_step "Clone $REPO_OVERRIDE -> $REPO_DIR" git clone "$REPO_OVERRIDE" "$REPO_DIR"
      cd "$REPO_DIR" 2>/dev/null || true
    elif [[ -d "$REPO_DIR/.git" ]]; then
      run_step "Updating existing clone at $REPO_DIR" git -C "$REPO_DIR" pull --ff-only || die "git pull failed in $REPO_DIR"
      cd "$REPO_DIR"
    else
      run_step "Cloning $REPO_OVERRIDE -> $REPO_DIR" git clone "$REPO_OVERRIDE" "$REPO_DIR" || die "git clone failed"
      cd "$REPO_DIR"
    fi
  else
    REPO_DIR="$(find_repo_root)" || die "could not locate the repo (no core/Cargo.toml found upward from this script). Run from inside the repo, or use --repo <url>, or drop --build-from-source to download prebuilt binaries."
    cd "$REPO_DIR"
  fi
  REPO_DIR="$(cd "$REPO_DIR" && pwd)"
  ORIGIN_URL="$(git -C "$REPO_DIR" remote get-url origin 2>/dev/null || true)"
  if [[ ! -f "$REPO_DIR/core/Cargo.toml" ]]; then
    $DRY_RUN || die "repo at $REPO_DIR has no core/Cargo.toml — not the catalyst-code repo?"
  fi
}

detect_version() {
  VERSION_DETECTED="$(grep -m1 '^version' "$REPO_DIR/core/Cargo.toml" 2>/dev/null | sed -E 's/.*"([^"]+)".*/\1/' || true)"
  if [[ -z "$VERSION_DETECTED" ]]; then VERSION_DETECTED="$VERSION"; fi
}

build_core() {
  run_step "Building Rust core (cargo --release)" \
    cargo build --release --manifest-path "$REPO_DIR/core/Cargo.toml" \
    || die "core build failed"
}

build_tui() {
  run_step "Building Go TUI (catcode)" \
    go -C "$REPO_DIR/tui" build -o "$REPO_DIR/tui/tui" \
    || die "TUI build failed (need go >= ${GO_MIN}?)"
}

install_bins_source() {
  local core_bin="$REPO_DIR/core/target/release/core"
  local tui_bin="$REPO_DIR/tui/tui"
  if ! $DRY_RUN; then
    [[ -f "$core_bin" ]] || die "core binary not found at $core_bin"
    [[ -f "$tui_bin" ]]  || die "TUI binary not found at $tui_bin"
  fi
  run_root "Creating $PREFIX" mkdir -p "$PREFIX"
  run_root "Installing catcode-core -> $PREFIX/catcode-core" install -m 0755 "$core_bin" "$PREFIX/catcode-core"
  run_root "Installing catcode       -> $PREFIX/catcode"       install -m 0755 "$tui_bin"  "$PREFIX/catcode"
}

build_web_source() {
  detect_runtime
  run_step --cwd "$REPO_DIR/sdk" "Installing SDK deps ($RT)" $RT install \
    || die "SDK dependency install failed"
  run_step --cwd "$REPO_DIR/sdk" "Building SDK (tsc)" $RT run build \
    || die "SDK build failed (sdk/dist/)"
  run_step --cwd "$REPO_DIR/web" "Installing web deps ($RT)" $RT install \
    || die "web dependency install failed"
  run_step --cwd "$REPO_DIR/web" "Building web (next build)" \
    env NEXT_TELEMETRY_DISABLED=1 $RT run build \
    || die "web build failed (next build)"
}

install_web_systemd_source() {
  local unit_dir="/etc/systemd/system"
  local unit="$unit_dir/$UNIT_NAME"
  local tmp; tmp="$(mktemp -p "$TMPDIR_SELF")"
  local exec_start="$RT_BIN run start -- --hostname $HOST"
  cat >"$tmp" <<EOF
[Unit]
Description=Catalyst Code Web Frontend (port $PORT)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$INSTALL_USER
WorkingDirectory=$REPO_DIR/web
Environment=NODE_ENV=production
Environment=PORT=$PORT
Environment=CATCODE_CORE=$PREFIX/catcode-core
ExecStart=$exec_start
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF
  if $DRY_RUN; then
    log_info "[dry-run] would write unit: $unit"
    sed 's/^/      /' "$tmp"
    return 0
  fi
  run_root "Creating $unit_dir" mkdir -p "$unit_dir"
  run_root "Installing unit file" install -m 0644 "$tmp" "$unit" || die "could not install unit"
  run_root "Reloading systemd" systemctl daemon-reload || die "daemon-reload failed"
  run_root "Enabling $UNIT_NAME" systemctl enable "$UNIT_NAME" || die "enable failed"
  run_root "Starting $UNIT_NAME" systemctl start "$UNIT_NAME" || die "start failed — check: journalctl -u $UNIT_NAME -e"
}

install_web_launchd_source() {
  local agents_dir="$HOME/Library/LaunchAgents"
  local plist="$agents_dir/${LAUNCHD_LABEL}.plist"
  local log_dir="$HOME/Library/Logs"
  local log_file="$log_dir/catalyst-code-web.log"
  local tmp; tmp="$(mktemp -p "$TMPDIR_SELF")"
  cat >"$tmp" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${RT_BIN}</string>
    <string>run</string>
    <string>start</string>
    <string>--</string>
    <string>--hostname</string>
    <string>${HOST}</string>
  </array>
  <key>WorkingDirectory</key>
  <string>${REPO_DIR}/web</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>NODE_ENV</key>
    <string>production</string>
    <key>PORT</key>
    <string>${PORT}</string>
    <key>CATCODE_CORE</key>
    <string>${PREFIX}/catcode-core</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>${log_file}</string>
  <key>StandardErrorPath</key>
  <string>${log_file}</string>
</dict>
</plist>
EOF
  if $DRY_RUN; then
    log_info "[dry-run] would write plist: $plist"
    sed 's/^/      /' "$tmp"
    return 0
  fi
  run_step "Creating $agents_dir" mkdir -p "$agents_dir"
  run_step "Creating $log_dir" mkdir -p "$log_dir"
  run_step "Installing plist" install -m 0644 "$tmp" "$plist" || die "could not install plist"
  launchctl unload "$plist" >/dev/null 2>&1 || true
  run_step "Loading $LAUNCHD_LABEL" launchctl load "$plist" || die "load failed — check: cat $log_file"
  log_ok "Web agent loaded (starts at login, auto-restarts on crash)"
}

install_web_service_source() {
  if [[ "$PLATFORM" == "Darwin" ]]; then
    install_web_launchd_source
  else
    install_web_systemd_source
  fi
}

restart_web_service_source() {
  if [[ "$PLATFORM" == "Darwin" ]]; then
    local plist="$HOME/Library/LaunchAgents/${LAUNCHD_LABEL}.plist"
    launchctl unload "$plist" >/dev/null 2>&1 || true
    run_step "Reloading $LAUNCHD_LABEL" launchctl load "$plist" || die "load failed — check: cat $HOME/Library/Logs/catalyst-code-web.log"
  else
    run_root "Reloading systemd" systemctl daemon-reload
    run_root "Restarting $UNIT_NAME" systemctl restart "$UNIT_NAME" || die "restart failed — check: journalctl -u $UNIT_NAME -e"
  fi
}

do_install_source() {
  phase "Checking dependencies (source build)"
  check_deps_source
  ensure_sudo
  resolve_repo
  detect_version

  log_info "Repo:     $REPO_DIR"
  log_info "Origin:   ${ORIGIN_URL:-(none — local copy)}"
  log_info "Prefix:   $PREFIX"
  $WITH_WEB && log_info "Web:      $UNIT_NAME on :$PORT ($HOST)"

  phase "Building Rust core (catcode-core)"
  build_core
  phase "Building Go TUI (catcode)"
  build_tui
  phase "Installing binaries"
  install_bins_source

  if $WITH_WEB; then
    phase "Building web frontend (SDK + Next.js)"
    build_web_source
    phase "Installing web service"
    install_web_service_source
  else
    log_info "Skipping web service (pass --with-web to install it)"
  fi

  save_state
  summary_install
}

do_update_source() {
  phase "Reading previous install state"
  log_info "Repo:     $REPO_DIR"
  log_info "Origin:   ${ORIGIN_URL:-(none)}"
  log_info "Prefix:   $PREFIX"
  [[ "${WEB_INSTALLED:-no}" == yes ]] && log_info "Web:      $UNIT_NAME on :$PORT"

  phase "Updating source (git)"
  if [[ ! -d "$REPO_DIR/.git" ]]; then
    if [[ -n "${ORIGIN_URL:-}" ]]; then
      die "repo at $REPO_DIR is not a git checkout. Re-clone with: bash install.sh --build-from-source --repo $ORIGIN_URL"
    fi
    die "repo at $REPO_DIR is not a git checkout and no origin URL recorded — cannot update."
  fi
  run_step "Pulling latest (git pull --ff-only)" git -C "$REPO_DIR" pull --ff-only \
    || die "git pull failed — resolve conflicts: cd $REPO_DIR && git status"

  cd "$REPO_DIR"
  detect_version
  log_info "Version:  $VERSION_DETECTED"
  ensure_sudo

  phase "Rebuilding Rust core"
  build_core
  phase "Rebuilding Go TUI"
  build_tui
  phase "Reinstalling binaries"
  install_bins_source

  if [[ "${WEB_INSTALLED:-no}" == yes ]]; then
    phase "Rebuilding web frontend"
    build_web_source
    phase "Restarting web service"
    restart_web_service_source
  fi

  save_state
  summary_update
}

# ── state file ───────────────────────────────────────────────
save_state() {
  local f="$STATE_FILE"
  local tmp; tmp="$(mktemp -p "$TMPDIR_SELF")"
  local web_flag="no"; $WITH_WEB && web_flag="yes"
  cat >"$tmp" <<EOF
# Catalyst Code installer state — written by install.sh
# (shell-sourcable; safe to read with 'source')
METHOD="$METHOD"
REPO_DIR="$REPO_DIR"
ORIGIN_URL="${ORIGIN_URL:-}"
PREFIX="$PREFIX"
PORT="$PORT"
HOST="$HOST"
RUNTIME="${RUNTIME:-}"
WEB_DIR="${WEB_DIR:-}"
WEB_INSTALLED="$web_flag"
UNIT_NAME="$UNIT_NAME"
INSTALL_USER="$INSTALL_USER"
VERSION="$VERSION_DETECTED"
INSTALLED_AT="$(date -u +%FT%TZ)"
EOF
  if $DRY_RUN; then
    log_info "[dry-run] would write state: $f"
    sed 's/^/      /' "$tmp"
    return 0
  fi
  run_root "Creating $(dirname "$f")" mkdir -p "$(dirname "$f")"
  run_root "Recording install state" install -m 0644 "$tmp" "$f" || die "could not write state file"
}

load_state() {
  [[ -f "$STATE_FILE" ]] || return 1
  # shellcheck source=/dev/null
  source "$STATE_FILE" 2>/dev/null || return 1
  return 0
}

# ── actions ──────────────────────────────────────────────────
do_install() {
  if $BUILD_FROM_SOURCE; then
    do_install_source
  else
    do_install_download
  fi
}

do_update() {
  phase "Reading previous install state"
  if ! load_state; then
    die "no previous install found at $STATE_FILE — run 'bash install.sh' first."
  fi
  if [[ "${METHOD:-download}" == "source" ]]; then
    do_update_source
  else
    do_update_download
  fi
}

do_uninstall() {
  phase "Reading install state"
  if load_state; then
    log_info "Found previous install (method: ${METHOD:-download}, repo/web-dir: ${WEB_DIR:-${REPO_DIR:-<none>}})"
  else
    log_warn "no state file at $STATE_FILE — attempting default paths"
  fi

  ensure_sudo

  phase "Stopping & removing web service"
  if [[ "$PLATFORM" == "Darwin" ]]; then
    local plist="$HOME/Library/LaunchAgents/${LAUNCHD_LABEL}.plist"
    if $DRY_RUN; then
      log_info "[dry-run] would unload + remove $plist"
    elif [[ -f "$plist" ]]; then
      launchctl unload "$plist" 2>/dev/null || true
      rm -f "$plist"
      log_ok "Removed launchd agent $LAUNCHD_LABEL"
    else
      log_info "No launchd agent at $plist (already removed?)"
    fi
  else
    root_do "Stop $UNIT_NAME"   systemctl stop "$UNIT_NAME"
    root_do "Disable $UNIT_NAME" systemctl disable "$UNIT_NAME"
    root_do "Remove unit file"  rm -f "/etc/systemd/system/$UNIT_NAME"
    root_do "Reload systemd"   systemctl daemon-reload
  fi

  phase "Removing binaries"
  root_do "Remove $PREFIX/catcode"       rm -f "$PREFIX/catcode"
  root_do "Remove $PREFIX/catcode-core"  rm -f "$PREFIX/catcode-core"

  phase "Cleaning up"
  if [[ -n "${WEB_DIR:-}" && -d "${WEB_DIR:-}" ]]; then
    root_do "Remove web bundle $WEB_DIR" rm -rf "$WEB_DIR"
  fi
  root_do "Remove state file" rm -f "$STATE_FILE"

  summary_uninstall
}

# ── summaries ───────────────────────────────────────────────
summary_install() {
  local web_line="(not installed — run with --with-web)"
  local svc_line=""
  if $WITH_WEB; then
    local svc_id="$UNIT_NAME"
    [[ "$PLATFORM" == "Darwin" ]] && svc_id="$LAUNCHD_LABEL (launchd)"
    web_line="http://${HOST}:${PORT}  (running as $svc_id)"
    svc_line="service:   $svc_id  (enabled, auto-restart)"
  fi
  print_box "✓  Installed  ${APP_NAME}  v${VERSION_DETECTED}" \
    "tui:       $PREFIX/catcode" \
    "core:      $PREFIX/catcode-core" \
    "web:       $web_line" \
    "$svc_line" \
    "update:    bash install.sh --update" \
    "uninstall: bash install.sh --uninstall" \
    "log:       ${LOG_FILE:-<disabled>}"
  log_info "Run the TUI with:  catcode"
  if $WITH_WEB; then
    if [[ "$PLATFORM" == "Darwin" ]]; then
      log_info "Web service logs:  tail -f $HOME/Library/Logs/catalyst-code-web.log"
    else
      log_info "Web service logs:  journalctl -u $UNIT_NAME -f"
    fi
    log_warn "Auth: ensure a key/login exists (~/.config/catalyst-code/settings.json) or set UMANS_API_KEY."
    if [[ "$HOST" != "127.0.0.1" ]]; then
      log_warn "Bound to $HOST — put a TLS reverse proxy in front for public use."
    fi
  fi
}

summary_update() {
  local web_line="(web service not installed)"
  [[ "${WEB_INSTALLED:-no}" == yes ]] && web_line="http://${HOST}:${PORT}  (restarted)"
  print_box "✓  Updated  ${APP_NAME}  v${VERSION_DETECTED}" \
    "tui:    $PREFIX/catcode" \
    "core:   $PREFIX/catcode-core" \
    "web:    $web_line" \
    "source: ${METHOD:-download} @ ${BASE_URL:-${REPO_DIR:-<unknown>}}"
  log_info "Run the TUI with:  catcode"
}

summary_uninstall() {
  print_box "✓  Uninstalled  ${APP_NAME}" \
    "removed: $PREFIX/catcode" \
    "removed: $PREFIX/catcode-core" \
    "removed: $UNIT_NAME (stopped + disabled)" \
    "removed: ${WEB_DIR:-<web bundle>}" \
    "removed: $STATE_FILE"
  if [[ "${METHOD:-}" == "source" && -n "${REPO_DIR:-}" ]]; then
    log_info "The cloned repo at $REPO_DIR was left untouched."
  fi
}

# ── main ────────────────────────────────────────────────────
main() {
  parse_args "$@"
  if ! $USE_COLOR; then
    C_RED=""; C_GREEN=""; C_YELLOW=""; C_CYAN=""; C_DIM=""; C_BOLD=""; C_RST=""
  fi
  setup_log
  # detect version early for the banner (only if a repo is present)
  if _d="$(find_repo_root 2>/dev/null)"; then VERSION_DETECTED="$(grep -m1 '^version' "$_d/core/Cargo.toml" 2>/dev/null | sed -E 's/.*"([^"]+)".*/\1/' || true)"; [[ -z "$VERSION_DETECTED" ]] && VERSION_DETECTED="$VERSION"; fi
  print_banner
  case "$ACTION" in
    install)   do_install ;;
    update)    do_update ;;
    uninstall) do_uninstall ;;
    *)         die "unknown action: $ACTION" ;;
  esac
}

main "$@"
