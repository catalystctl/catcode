#!/usr/bin/env bash
# ============================================================
# Catalyst Code — Installer  v1.0.0
# Platform: Linux (systemd)
#
# Installs the catalyst-code TUI (`catcode`) + core (`catcode-core`) to your PATH,
# and can optionally install the Next.js web frontend as a systemd service
# that stays online 24/7 (default port 49283).
#
# Update mode records how you obtained the source (a git clone, or a remote
# URL) so `--update` can `git pull` + rebuild + reinstall.
#
# Usage:
#   bash install.sh                       # install TUI + core
#   bash install.sh --with-web            # also install the web service
#   bash install.sh --repo <url>          # clone first, then install
#   bash install.sh --update              # git pull + rebuild + reinstall
#   bash install.sh --uninstall           # remove everything
#
# Options:
#   --with-web            install the web frontend service
#   --repo <url>          clone <url> first (then install from it)
#   --prefix <path>       binary install dir   (default: /usr/local/bin)
#   --port <n>            web service port      (default: 49283)
#   --host <h>            web bind host         (default: 0.0.0.0)
#   --log-file <path>     write a log here      (default: ~/catalyst-code-install.log)
#   --no-log             disable logging
#   --no-color           disable ANSI colors
#   --dry-run            print the plan, execute nothing
#   -h, --help           show this help
# ============================================================
set -euo pipefail

# ── constants ────────────────────────────────────────────────
APP_NAME="Catalyst Code"
VERSION="1.0.0"
DEFAULT_PREFIX="/usr/local/bin"
DEFAULT_PORT="49283"
DEFAULT_HOST="0.0.0.0"
STATE_FILE="/etc/catalyst-code/installer.state"
UNIT_NAME="catalyst-code-web.service"
GO_MIN="1.24.2"

# ── option defaults ──────────────────────────────────────────
ACTION="install"
DRY_RUN=false
WITH_WEB=false
REPO_OVERRIDE=""
PREFIX="$DEFAULT_PREFIX"
PORT="$DEFAULT_PORT"
HOST="$DEFAULT_HOST"
LOG_FILE="${HOME}/catalyst-code-install.log"
NO_COLOR_FLAG=false
LOG_ENABLED=false

# ── runtime state ────────────────────────────────────────────
SUDO=""
RUNTIME=""        # bun | npm
RT_BIN=""         # absolute path to bun/npm
RT=""             # runtime word for "run"/"install" invocations
REPO_DIR=""
ORIGIN_URL=""
INSTALL_USER=""
VERSION_DETECTED="$VERSION"

# ── temp dir + cleanup ───────────────────────────────────────
TMPDIR_SELF=""
cleanup() {
  [[ -n "${TMPDIR_SELF:-}" && -d "${TMPDIR_SELF:-}" ]] && rm -rf "$TMPDIR_SELF"
}
trap cleanup EXIT
TMPDIR_SELF="$(mktemp -d -t umans-inst.XXXXXX 2>/dev/null || mktemp -d)"

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

# root_do "message" command [args...] — tolerant (no spinner, ignores failure).
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

# ── banner ───────────────────────────────────────────────────
print_banner() {
  print_box "Catalyst Code  —  installer v${VERSION_DETECTED}" \
    "TUI (catcode) + core (catcode-core) -> PATH" \
    "optional 24/7 web service (Next.js)" \
    "scope: system-wide   |   platform: linux (systemd)"
  printf "  ${C_DIM}mode: %s   |   dry-run: %s${C_RST}\n\n" "$ACTION" "$DRY_RUN"
}

# ── box printer ──────────────────────────────────────────────
# print_box TITLE  line1 line2 ...
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

# ── arg parsing ──────────────────────────────────────────────
usage() {
  awk 'NR==1{next} /^#/{print; next} {exit}' "$0" | sed 's/^# \{0,1\}//'
  exit 0
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --install)   ACTION="install" ;;
      --update|--upgrade) ACTION="update" ;;
      --uninstall) ACTION="uninstall" ;;
      --dry-run)   DRY_RUN=true ;;
      --with-web)  WITH_WEB=true ;;
      --repo)      [[ $# -ge 2 ]] || die "--repo requires a URL"; REPO_OVERRIDE="$2"; shift ;;
      --prefix)    [[ $# -ge 2 ]] || die "--prefix requires a path"; PREFIX="$2"; shift ;;
      --port)      [[ $# -ge 2 ]] || die "--port requires a number"; PORT="$2"; shift ;;
      --host)      [[ $# -ge 2 ]] || die "--host requires a value"; HOST="$2"; shift ;;
      --log-file)  [[ $# -ge 2 ]] || die "--log-file requires a path"; LOG_FILE="$2"; shift ;;
      --no-log)    LOG_FILE="" ;;
      --no-color)  NO_COLOR_FLAG=true ;;
      -h|--help)   usage ;;
      *)           die "unknown option: $1 (try --help)" ;;
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
  _log "action=$ACTION dry_run=$DRY_RUN with_web=$WITH_WEB prefix=$PREFIX port=$PORT host=$HOST"
}

# ── dependency checks ────────────────────────────────────────
check_deps() {
  local missing=()
  have cargo || missing+=("cargo (Rust toolchain — https://rustup.rs)")
  have go    || missing+=("go (>= ${GO_MIN} — https://go.dev/dl/)")
  if have go; then
    local gv
    gv="$(go version 2>/dev/null | sed -E 's/.*go([0-9]+\.[0-9]+(\.[0-9]+)?).*/\1/' || echo 0)"
    ver_ge "$GO_MIN" "$gv" || missing+=("go >= ${GO_MIN} (have ${gv})")
  fi
  if $WITH_WEB || [[ "$ACTION" == "uninstall" ]]; then :; fi
  if $WITH_WEB; then
    have bun || have npm || missing+=("bun or npm (for the web build — https://bun.sh)")
  fi
  if [[ ${#missing[@]} -gt 0 ]]; then
    printf "\n  ${C_BOLD}${C_RED}Missing dependencies:${C_RST}\n" >&2
    local m
    for m in "${missing[@]}"; do printf "    ${C_RED}• %s${C_RST}\n" "$m" >&2; done
    die "install the dependencies above, then re-run."
  fi
  log_ok "Dependencies present (cargo, go${WITH_WEB:+, node/bun})"
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
    log_info "System-wide install — authenticating with sudo..."
    sudo -v || die "sudo authentication failed"
  fi
}

# ── runtime detection (bun vs npm) ───────────────────────────
detect_runtime() {
  if have bun; then
    RUNTIME="bun"; RT_BIN="$(command -v bun)"; RT="bun"
  elif have npm; then
    RUNTIME="npm"; RT_BIN="$(command -v npm)"; RT="npm"
    have node || die "node not found (npm requires it)"
  else
    die "neither bun nor npm found — install one to build the web frontend (https://bun.sh)"
  fi
  log_ok "Web runtime: $RUNTIME ($RT_BIN)"
}

# ── repo resolution ──────────────────────────────────────────
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
    REPO_DIR="$(find_repo_root)" || die "could not locate the repo (no core/Cargo.toml found upward from this script). Run from inside the repo, or use --repo <url>."
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
  [[ -z "$VERSION_DETECTED" ]] && VERSION_DETECTED="$VERSION"
}

# ── build steps ──────────────────────────────────────────────
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

install_bins() {
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

build_web() {
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

install_web_service() {
  detect_runtime
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
# NOTE: runs next start in production mode. For public exposure, put a
# reverse proxy (caddy/nginx) with TLS in front and bind --host 127.0.0.1.

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

restart_web_service() {
  run_root "Reloading systemd" systemctl daemon-reload
  run_root "Restarting $UNIT_NAME" systemctl restart "$UNIT_NAME" || die "restart failed — check: journalctl -u $UNIT_NAME -e"
}

# ── state file ───────────────────────────────────────────────
save_state() {
  local f="$STATE_FILE"
  local tmp; tmp="$(mktemp -p "$TMPDIR_SELF")"
  local web_flag="no"; $WITH_WEB && web_flag="yes"
  cat >"$tmp" <<EOF
# Catalyst Code installer state — written by install.sh
# (shell-sourcable; safe to read with 'source')
REPO_DIR="$REPO_DIR"
ORIGIN_URL="${ORIGIN_URL:-}"
PREFIX="$PREFIX"
PORT="$PORT"
HOST="$HOST"
RUNTIME="${RUNTIME:-}"
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
  phase "Checking dependencies"
  check_deps
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
  install_bins

  if $WITH_WEB; then
    phase "Building web frontend (SDK + Next.js)"
    build_web
    phase "Installing web service"
    install_web_service
  else
    log_info "Skipping web service (pass --with-web to install it)"
  fi

  save_state
  summary_install
}

do_update() {
  phase "Reading previous install state"
  if ! load_state; then
    die "no previous install found at $STATE_FILE — run 'bash install.sh' first."
  fi
  log_info "Repo:     $REPO_DIR"
  log_info "Origin:   ${ORIGIN_URL:-(none)}"
  log_info "Prefix:   $PREFIX"
  [[ "${WEB_INSTALLED:-no}" == yes ]] && log_info "Web:      $UNIT_NAME on :$PORT"

  phase "Updating source (git)"
  if [[ ! -d "$REPO_DIR/.git" ]]; then
    if [[ -n "${ORIGIN_URL:-}" ]]; then
      die "repo at $REPO_DIR is not a git checkout. Re-clone with: bash install.sh --repo $ORIGIN_URL"
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
  install_bins

  if [[ "${WEB_INSTALLED:-no}" == yes ]]; then
    phase "Rebuilding web frontend"
    build_web
    phase "Restarting web service"
    restart_web_service
  fi

  save_state
  summary_update
}

do_uninstall() {
  phase "Reading install state"
  if load_state; then
    log_info "Found previous install (repo: $REPO_DIR)"
  else
    log_warn "no state file at $STATE_FILE — attempting default paths"
  fi

  ensure_sudo

  phase "Stopping & removing web service"
  root_do "Stop $UNIT_NAME"   systemctl stop "$UNIT_NAME"
  root_do "Disable $UNIT_NAME" systemctl disable "$UNIT_NAME"
  root_do "Remove unit file"  rm -f "/etc/systemd/system/$UNIT_NAME"
  root_do "Reload systemd"   systemctl daemon-reload

  phase "Removing binaries"
  root_do "Remove $PREFIX/catcode-core" rm -f "$PREFIX/catcode-core"
  root_do "Remove $PREFIX/catcode"       rm -f "$PREFIX/catcode"

  phase "Cleaning up"
  root_do "Remove state file" rm -f "$STATE_FILE"

  summary_uninstall
}

# ── summaries ───────────────────────────────────────────────
summary_install() {
  local web_line="(not installed — run with --with-web)"
  local svc_line=""
  if $WITH_WEB; then
    web_line="http://${HOST}:${PORT}  (running as $UNIT_NAME)"
    svc_line="service:   $UNIT_NAME  (enabled, auto-restart)"
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
  $WITH_WEB && log_info "Web service logs:  journalctl -u $UNIT_NAME -f"
  $WITH_WEB && log_warn "Auth: ensure a key/login exists (~/.config/catalyst-code/settings.json) or set UMANS_API_KEY."
  $WITH_WEB && [[ "$HOST" != "127.0.0.1" ]] && log_warn "Bound to $HOST — put a TLS reverse proxy in front for public use."
}

summary_update() {
  local web_line="(web service not installed)"
  [[ "${WEB_INSTALLED:-no}" == yes ]] && web_line="http://${HOST}:${PORT}  (restarted)"
  print_box "✓  Updated  ${APP_NAME}  v${VERSION_DETECTED}" \
    "tui:    $PREFIX/catcode" \
    "core:   $PREFIX/catcode-core" \
    "web:    $web_line" \
    "source: git pull @ $REPO_DIR"
  log_info "Run the TUI with:  catcode"
}

summary_uninstall() {
  print_box "✓  Uninstalled  ${APP_NAME}" \
    "removed: $PREFIX/catcode" \
    "removed: $PREFIX/catcode-core" \
    "removed: $UNIT_NAME (stopped + disabled)" \
    "removed: $STATE_FILE"
  log_info "The cloned repo at ${REPO_DIR:-<unknown>} was left untouched."
}

# ── main ────────────────────────────────────────────────────
main() {
  parse_args "$@"
  setup_colors 2>/dev/null || true
  # re-evaluate colors after flag parse (NO_COLOR_FLAG may have changed)
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
