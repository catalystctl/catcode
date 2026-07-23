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
#   bash install.sh                       # interactive menu + optional settings prompts
#   bash install.sh --install             # download + install catcode (skip menu)
#   bash install.sh --with-web             # …also install the web service
#   bash install.sh --add-web              # add the web service to an existing install
#   bash install.sh --update               # re-download latest + reinstall
#   bash install.sh --reinstall            # reinstall the currently-installed version
#   bash install.sh --uninstall            # remove everything
#   bash install.sh --status               # show the current install state
#   bash install.sh --version 0.2.0        # pin a version
#   bash install.sh --base-url <url>       # download from a mirror (not GitHub)
#   bash install.sh --expose intranet      # CORS/exposure: local|intranet|public
#   bash install.sh --build-from-source    # fall back to cargo+go+next build
#
# Options:
#   --install             install (skip the interactive menu)
#   --with-web            install the web frontend service
#   --add-web             add the web service to an existing install
#   --update              re-download latest + reinstall
#   --reinstall           reinstall the currently-installed version
#   --uninstall           remove everything
#   --status              show the current install state
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
#   --expose <mode>       CORS/exposure mode: local|intranet|public (default: intranet)
#                         local=loopback only; intranet=LAN (auto-detected origin);
#                         public=internet (requires --origin, use a TLS reverse proxy)
#   --origin <url>        canonical origin (CATCODE_WEB_ORIGIN); overrides --expose derivation
#                         e.g. https://code.example.com or http://192.168.1.50:49283
#   --trusted-origins <list>  additional trusted origins for better-auth CSRF (BETTER_AUTH_TRUSTED_ORIGINS,
#                         comma-separated; for a proxy/multi-domain setup, e.g.
#                         https://code.example.com,https://staging.example.com)
#   --workspace <path>    default workspace the core opens (CATALYST_CODE_WORKSPACE)
#   --shell <path>        terminal shell for the web terminal (SHELL; default: platform)
#   --idle-gc-ms <n>      idle live-session GC interval in ms (UMANS_WEB_IDLE_GC_MS; 0=disable)
#   --installer-url <u>   self-update install.sh URL (CATCODE_INSTALLER_URL)
#   --windows-installer-url <u>  self-update install.ps1 URL (CATCODE_WINDOWS_INSTALLER_URL)
#   --skip-service        install web files only (do not write/start systemd/launchd)
#   --force-web-service   replace a non-installer-managed catalyst-code-web unit
#   --log-file <path>     write a log here      (default: ~/catalyst-code-install.log)
#   --no-log              disable logging
#   --no-color           disable ANSI colors
#   (interactive menu also offers Customize install settings: prefix/web-dir/port/expose/origin/version/base-url
#    + an optional advanced-settings sub-prompt for workspace/shell/idle-gc/installer-urls)
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
DEFAULT_EXPOSE="intranet"   # local | intranet | public (see --expose)
DEFAULT_IDLE_GC_MS="7200000"  # 2h — UMANS_WEB_IDLE_GC_MS
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
SKIP_SERVICE=false
FORCE_WEB_SERVICE=false
VERSION_OVERRIDE=""
BASE_URL_OVERRIDE=""
WEB_DIR_OVERRIDE=""
REPO_OVERRIDE=""
PREFIX="$DEFAULT_PREFIX"
PORT="$DEFAULT_PORT"
HOST=""               # resolved by resolve_expose() from --host / --expose
# ── web-service env knobs (all adjustable; empty = omit from unit) ──
EXPOSE_MODE="$DEFAULT_EXPOSE"
ORIGIN_OVERRIDE=""      # --origin  (CATCODE_WEB_ORIGIN)
WORKSPACE=""           # --workspace (CATALYST_CODE_WORKSPACE)
TERM_SHELL=""          # --shell   (stamped as SHELL)
IDLE_GC_MS=""          # --idle-gc-ms (UMANS_WEB_IDLE_GC_MS)
INSTALLER_URL=""       # --installer-url (CATCODE_INSTALLER_URL)
WIN_INSTALLER_URL=""   # --windows-installer-url (CATCODE_WINDOWS_INSTALLER_URL)
TRUSTED_ORIGINS=""    # --trusted-origins (BETTER_AUTH_TRUSTED_ORIGINS, comma-separated)
# CLI-intent captures: NOT persisted in state, so they survive load_state's
# `source` and re-apply on update/reinstall/add-web (where state clobbers the
# effective vars). parse_args sets both the effective var AND its WANT_* twin.
WANT_EXPOSE=""; WANT_PORT=""; WANT_HOST=""; WANT_ORIGIN=""; WANT_WORKSPACE=""; WANT_SHELL=""
WANT_IDLE_GC=""; WANT_INSTALLER_URL=""; WANT_WIN_INSTALLER_URL=""; WANT_TRUSTED_ORIGINS=""
ORIGIN=""              # resolved canonical origin (CATCODE_WEB_ORIGIN value)
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

# ── web-service env: exposure mode + origin + bind ──────────
# Derive the primary non-loopback IPv4 of this host (for --expose intranet's
# auto-origin). Returns the IP on stdout, empty + non-zero on failure.
detect_lan_ip() {
  local ip="" iface=""
  case "$PLATFORM" in
    Linux)
      # src IP used to reach the internet — most reliable primary-LAN address.
      ip="$(ip -4 -o route get 1.1.1.1 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i=="src"){print $(i+1); exit}}' | head -1)"
      [[ -z "$ip" ]] && ip="$(hostname -I 2>/dev/null | awk '{print $1}')"
      ;;
    Darwin)
      iface="$(route -n get default 2>/dev/null | awk '/interface:/{print $2; exit}')"
      [[ -n "$iface" ]] && ip="$(ipconfig getifaddr "$iface" 2>/dev/null)"
      [[ -z "$ip" ]] && ip="$(ipconfig getifaddr en0 2>/dev/null)"
      ;;
  esac
  # validate: present, IPv4-looking, not loopback / not-any.
  if [[ -n "$ip" && "$ip" != 127.* && "$ip" != 0.* && "$ip" != 169.254.* ]]; then
    printf '%s' "$ip"; return 0
  fi
  return 1
}

# Resolve bind HOST + canonical ORIGIN from EXPOSE_MODE (+ overrides).
#   --host   (WANT_HOST)            wins over the mode-derived bind
#   --origin (ORIGIN_OVERRIDE)       wins over the mode-derived origin
# Precedence for HOST:  WANT_HOST → mode (local=127.0.0.1, else 0.0.0.0).
# Precedence for ORIGIN: ORIGIN_OVERRIDE → mode derivation.
resolve_expose() {
  if [[ -n "${WANT_HOST:-}" ]]; then
    HOST="$WANT_HOST"
  else
    case "$EXPOSE_MODE" in
      local)    HOST="127.0.0.1" ;;
      intranet|public) HOST="0.0.0.0" ;;
      *) die "unknown --expose mode: $EXPOSE_MODE (use local|intranet|public)" ;;
    esac
  fi
  if [[ -n "${ORIGIN_OVERRIDE:-}" ]]; then
    ORIGIN="$ORIGIN_OVERRIDE"
  else
    case "$EXPOSE_MODE" in
      local)
        ORIGIN="http://localhost:${PORT}"
        ;;
      intranet)
        local lan_ip
        if lan_ip="$(detect_lan_ip)"; then
          ORIGIN="http://${lan_ip}:${PORT}"
        else
          ORIGIN="http://localhost:${PORT}"
          log_warn "could not auto-detect a LAN IP for --expose intranet; non-loopback auth will fail until you set --origin http://<lan-ip>:${PORT}"
        fi
        ;;
      public)
        die "--expose public requires --origin <url> (your public/tunnel URL, e.g. https://code.example.com)"
        ;;
    esac
  fi
}

# Apply CLI overrides that load_state may have clobbered (WANT_* survive
# sourcing because they are NOT written to the state file), default the
# exposure mode, then derive bind+origin. Call once in each install action
# dispatcher, AFTER state has been loaded (for update/reinstall/add-web) and
# AFTER parse_args/prompt_install_options (for fresh install).
finalize_web_env() {
  # Bind/origin only matter for the web service. Skip for TUI-only installs so a
  # stray --expose/--origin can't crash a non-web install, and so we don't die
  # on `--expose public` when no web is being installed.
  if ! $WITH_WEB; then return 0; fi
  [[ -n "${WANT_PORT:-}" ]]           && PORT="$WANT_PORT"
  [[ -n "${WANT_HOST:-}" ]]          && HOST="$WANT_HOST"
  [[ -n "${WANT_EXPOSE:-}" ]]        && EXPOSE_MODE="$WANT_EXPOSE"
  [[ -n "${WANT_ORIGIN:-}" ]]        && ORIGIN_OVERRIDE="$WANT_ORIGIN"
  [[ -n "${WANT_WORKSPACE:-}" ]]    && WORKSPACE="$WANT_WORKSPACE"
  [[ -n "${WANT_SHELL:-}" ]]         && TERM_SHELL="$WANT_SHELL"
  [[ -n "${WANT_IDLE_GC:-}" ]]       && IDLE_GC_MS="$WANT_IDLE_GC"
  [[ -n "${WANT_INSTALLER_URL:-}" ]] && INSTALLER_URL="$WANT_INSTALLER_URL"
  [[ -n "${WANT_WIN_INSTALLER_URL:-}" ]] && WIN_INSTALLER_URL="$WANT_WIN_INSTALLER_URL"
  [[ -n "${WANT_TRUSTED_ORIGINS:-}" ]] && TRUSTED_ORIGINS="$WANT_TRUSTED_ORIGINS"
  [[ -z "${EXPOSE_MODE:-}" ]] && EXPOSE_MODE="$DEFAULT_EXPOSE"
  # (Re-)derive bind+origin when a CLI override is present, or when no resolved
  # origin exists yet (fresh install / old state). A plain update keeps the
  # previously-resolved ORIGIN+HOST so a stable install doesn't churn.
  if [[ -n "${ORIGIN_OVERRIDE:-}" || -n "${WANT_EXPOSE:-}" || -n "${WANT_HOST:-}" || -z "${ORIGIN:-}" ]]; then
    resolve_expose
  fi
}

# Emit `Environment=KEY=value` lines for the systemd unit (one per line).
systemd_env_lines() {
  printf 'Environment=NODE_ENV=production\n'
  printf 'Environment=PORT=%s\n' "$PORT"
  printf 'Environment=HOSTNAME=%s\n' "$HOST"
  printf 'Environment=CATCODE_CORE=%s\n' "$PREFIX/catcode-core"
  [[ -n "$ORIGIN" ]]            && printf 'Environment=CATCODE_WEB_ORIGIN=%s\n' "$ORIGIN"
  [[ -n "$WORKSPACE" ]]         && printf 'Environment=CATALYST_CODE_WORKSPACE=%s\n' "$WORKSPACE"
  [[ -n "$TERM_SHELL" ]]        && printf 'Environment=SHELL=%s\n' "$TERM_SHELL"
  [[ -n "$IDLE_GC_MS" ]]        && printf 'Environment=UMANS_WEB_IDLE_GC_MS=%s\n' "$IDLE_GC_MS"
  [[ -n "$INSTALLER_URL" ]]     && printf 'Environment=CATCODE_INSTALLER_URL=%s\n' "$INSTALLER_URL"
  [[ -n "$WIN_INSTALLER_URL" ]] && printf 'Environment=CATCODE_WINDOWS_INSTALLER_URL=%s\n' "$WIN_INSTALLER_URL"
  [[ -n "$TRUSTED_ORIGINS" ]] && printf 'Environment=BETTER_AUTH_TRUSTED_ORIGINS=%s\n' "$TRUSTED_ORIGINS"
}

# Emit launchd <key>/<string> pairs (4-space indent) for the EnvironmentVariables dict.
launchd_env_pairs() {
  printf '    <key>NODE_ENV</key>\n    <string>production</string>\n'
  printf '    <key>PORT</key>\n    <string>%s</string>\n' "$PORT"
  printf '    <key>HOSTNAME</key>\n    <string>%s</string>\n' "$HOST"
  printf '    <key>CATCODE_CORE</key>\n    <string>%s</string>\n' "$PREFIX/catcode-core"
  [[ -n "$ORIGIN" ]]            && printf '\n    <key>CATCODE_WEB_ORIGIN</key>\n    <string>%s</string>' "$ORIGIN"
  [[ -n "$WORKSPACE" ]]         && printf '\n    <key>CATALYST_CODE_WORKSPACE</key>\n    <string>%s</string>' "$WORKSPACE"
  [[ -n "$TERM_SHELL" ]]        && printf '\n    <key>SHELL</key>\n    <string>%s</string>' "$TERM_SHELL"
  [[ -n "$IDLE_GC_MS" ]]        && printf '\n    <key>UMANS_WEB_IDLE_GC_MS</key>\n    <string>%s</string>' "$IDLE_GC_MS"
  [[ -n "$INSTALLER_URL" ]]     && printf '\n    <key>CATCODE_INSTALLER_URL</key>\n    <string>%s</string>' "$INSTALLER_URL"
  [[ -n "$WIN_INSTALLER_URL" ]] && printf '\n    <key>CATCODE_WINDOWS_INSTALLER_URL</key>\n    <string>%s</string>' "$WIN_INSTALLER_URL"
  [[ -n "$TRUSTED_ORIGINS" ]] && printf '\n    <key>BETTER_AUTH_TRUSTED_ORIGINS</key>\n    <string>%s</string>' "$TRUSTED_ORIGINS"
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
      --add-web)            ACTION="add-web" ;;
      --reinstall)          ACTION="reinstall" ;;
      --status)             ACTION="status" ;;
      --dry-run)            DRY_RUN=true ;;
      --with-web)           WITH_WEB=true ;;
      --skip-service)       SKIP_SERVICE=true ;;
      --force-web-service)  FORCE_WEB_SERVICE=true ;;
      --build-from-source)  BUILD_FROM_SOURCE=true; METHOD="source" ;;
      --version)            [[ $# -ge 2 ]] || die "--version requires a value"; VERSION_OVERRIDE="$2"; shift ;;
      --base-url)           [[ $# -ge 2 ]] || die "--base-url requires a URL"; BASE_URL_OVERRIDE="$2"; shift ;;
      --web-dir)            [[ $# -ge 2 ]] || die "--web-dir requires a path"; WEB_DIR_OVERRIDE="$2"; shift ;;
      --repo)               [[ $# -ge 2 ]] || die "--repo requires a URL"; REPO_OVERRIDE="$2"; shift ;;
      --prefix)             [[ $# -ge 2 ]] || die "--prefix requires a path"; PREFIX="$2"; shift ;;
      --port)               [[ $# -ge 2 ]] || die "--port requires a number"; PORT="$2"; WANT_PORT="$2"; shift ;;
      --host)               [[ $# -ge 2 ]] || die "--host requires a value"; WANT_HOST="$2"; shift ;;
      --expose)             [[ $# -ge 2 ]] || die "--expose requires a mode"; case "$2" in local|intranet|public) ;; *) die "--expose must be local|intranet|public (got: $2)";; esac; EXPOSE_MODE="$2"; WANT_EXPOSE="$2"; shift ;;
      --origin)             [[ $# -ge 2 ]] || die "--origin requires a URL"; ORIGIN_OVERRIDE="$2"; WANT_ORIGIN="$2"; shift ;;
      --workspace)          [[ $# -ge 2 ]] || die "--workspace requires a path"; WORKSPACE="$2"; WANT_WORKSPACE="$2"; shift ;;
      --shell)              [[ $# -ge 2 ]] || die "--shell requires a path"; TERM_SHELL="$2"; WANT_SHELL="$2"; shift ;;
      --idle-gc-ms)         [[ $# -ge 2 ]] || die "--idle-gc-ms requires a number"; IDLE_GC_MS="$2"; WANT_IDLE_GC="$2"; shift ;;
      --installer-url)      [[ $# -ge 2 ]] || die "--installer-url requires a URL"; INSTALLER_URL="$2"; WANT_INSTALLER_URL="$2"; shift ;;
      --windows-installer-url) [[ $# -ge 2 ]] || die "--windows-installer-url requires a URL"; WIN_INSTALLER_URL="$2"; WANT_WIN_INSTALLER_URL="$2"; shift ;;
      --trusted-origins)    [[ $# -ge 2 ]] || die "--trusted-origins requires a comma-separated list"; TRUSTED_ORIGINS="$2"; WANT_TRUSTED_ORIGINS="$2"; shift ;;
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
  _log "action=$ACTION dry_run=$DRY_RUN build_from_source=$BUILD_FROM_SOURCE with_web=$WITH_WEB prefix=$PREFIX port=$PORT host=$HOST expose=$EXPOSE_MODE origin=${ORIGIN_OVERRIDE:-<derived>}"
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

# ── runtime detection ────────────────────────────────────────
# mode=run   (default): require Node to execute the prebuilt start.js
# mode=build: prefer bun/npm for `install` / `run build` (source path)
detect_runtime() {
  local mode="${1:-run}"
  if [[ "$mode" == "build" ]]; then
    if have bun; then
      RUNTIME="bun"; RT_BIN="$(command -v bun)"; RT="bun"
    elif have npm; then
      RUNTIME="npm"; RT_BIN="$(command -v npm)"; RT="npm"
      have node || die "node not found (npm requires it)"
    else
      die "neither bun nor npm found — install one to BUILD the web frontend (https://bun.sh or https://nodejs.org)"
    fi
  else
    # Authentication uses node:sqlite, so Bun is not a compatible server
    # runtime even though it can install dependencies and run unit tests.
    if have node; then
      RUNTIME="node"; RT_BIN="$(command -v node)"; RT="node"
      local node_ver; node_ver="$(node -v | tr -d 'v')"
      if ! ver_ge "22.13.0" "$node_ver"; then
        die "Node.js >= 22.13.0 is required (found v${node_ver}); the web frontend uses node:sqlite"
      fi
    else
      die "Node.js >= 22.13.0 is required to run the web frontend (https://nodejs.org)"
    fi
  fi
  log_ok "Web runtime ($mode): $RUNTIME ($RT_BIN)"
}

# ════════════════════════════════════════════════════════════
# DOWNLOAD PATH (default — no compile)
# ════════════════════════════════════════════════════════════

detect_os_tag() {
  case "$PLATFORM" in
    Linux)  OS_TAG="linux" ;;
    Darwin) OS_TAG="macos" ;;
    *)      die "install.sh supports Linux and macOS only (this is '$PLATFORM'). Windows users: see install.ps1 (-WithWeb for the web service)" ;;
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
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$f" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$f" | awk '{print $1}')"
  else
    die "missing sha256sum or shasum (needed to verify downloads)"
  fi
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
# Asset is the universal cross-platform tarball (same on Linux/macOS/Windows):
#   catcode-web-<ver>.tar.gz
install_web_download() {
  detect_runtime run
  resolve_web_dir
  # Refuse early — before downloading a large tarball or touching WEB_DIR —
  # when a custom unit would be overwritten.
  if ! $SKIP_SERVICE; then
    protect_existing_web_service
  fi
  local web_asset="catcode-web-${VER}.tar.gz"
  fetch_asset "$web_asset"
  run_root "Creating $WEB_DIR" mkdir -p "$WEB_DIR"
  # Clean stale contents so an update doesn't leave old chunks.
  # Must run as root — default WEB_DIR is /opt/... and is root-owned.
  if ! $DRY_RUN; then
    run_root "Clearing $WEB_DIR" bash -c "find \"$WEB_DIR\" -mindepth 1 -delete"
  fi
  run_root "Extracting web bundle -> $WEB_DIR" tar xzf "$TMPDIR_SELF/$web_asset" -C "$WEB_DIR"
  if ! $DRY_RUN; then
    # Service runs as INSTALL_USER; make the tree readable/writable for sqlite etc.
    if [[ -n "${INSTALL_USER:-}" && "$INSTALL_USER" != "root" ]]; then
      run_root "Setting ownership of $WEB_DIR to $INSTALL_USER" chown -R "$INSTALL_USER:" "$WEB_DIR"
    fi
    # Always stamp the installed release commit into version.json (UI + /api/version).
    write_web_version_json "$WEB_DIR" "$VER" "release"
    validate_web_bundle "$WEB_DIR"
  fi
  if $SKIP_SERVICE; then
    log_info "Skipping web service install (--skip-service)"
    return 0
  fi
  install_web_service_download
}

# Write version.json so the web UI knows which git commit this install is.
# commit: short SHA / release tag; source: release|source|dev
write_web_version_json() {
  local dir="$1"
  local commit="$2"
  local source="${3:-release}"
  local commit_full="$commit"
  local dirty="false"
  local built_at
  built_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  # When installing from a live checkout, prefer the real git SHA + dirty bit.
  if [[ -n "${REPO_DIR:-}" && -d "${REPO_DIR}/.git" ]]; then
    commit_full="$(git -C "$REPO_DIR" rev-parse HEAD 2>/dev/null || echo "$commit")"
    commit="$(git -C "$REPO_DIR" rev-parse --short HEAD 2>/dev/null || echo "$commit")"
    if [[ -n "$(git -C "$REPO_DIR" status --porcelain 2>/dev/null || true)" ]]; then
      dirty="true"
    fi
  fi
  local tmp; tmp="$(mktemp -p "$TMPDIR_SELF")"
  cat >"$tmp" <<EOF
{
  "commit": "${commit}",
  "commitFull": "${commit_full}",
  "dirty": ${dirty},
  "builtAt": "${built_at}",
  "source": "${source}"
}
EOF
  if $DRY_RUN; then
    log_info "[dry-run] would write version.json -> $dir (commit=$commit source=$source)"
    return 0
  fi
  run_root "Writing version.json (commit $commit)" install -m 0644 "$tmp" "$dir/version.json"
  if [[ -d "$dir/.next" ]]; then
    run_root "Writing .next/version.json" install -m 0644 "$tmp" "$dir/.next/version.json"
  fi
  if [[ -n "${INSTALL_USER:-}" && "$INSTALL_USER" != "root" ]]; then
    run_root "Owning version.json" chown "$INSTALL_USER:" "$dir/version.json" 2>/dev/null || true
    [[ -f "$dir/.next/version.json" ]] && run_root "Owning .next/version.json" chown "$INSTALL_USER:" "$dir/.next/version.json" 2>/dev/null || true
  fi
  log_ok "Web version: $commit ($source)"
}

# Refuse to ship/start a broken web tarball (e.g. unflattened monorepo standalone).
validate_web_bundle() {
  local dir="$1"
  [[ -f "$dir/start.js" ]] || die "web bundle missing start.js (extraction failed?)"
  [[ -f "$dir/server.js" ]] || die "web bundle missing server.js"
  [[ -f "$dir/package.json" ]] || die "web bundle missing package.json (incomplete release artifact)"
  [[ -f "$dir/.next/BUILD_ID" ]] || die "web bundle missing .next/BUILD_ID (incomplete release artifact)"
  [[ -f "$dir/version.json" ]] || die "web bundle missing version.json (git commit not embedded — rebuild with current release-web.sh)"
  if [[ -f "$dir/web/server.js" || -d "$dir/web/node_modules" ]]; then
    die "web bundle has nested web/ layout — this release artifact was packed incorrectly.
  Re-run with a newer release (after release-web.sh flatten fix), or build locally with
  ./release-web.sh and pass --base-url / --version pointing at that artifact."
  fi
  local req
  for req in next ws zigpty better-auth; do
    [[ -f "$dir/node_modules/$req/package.json" ]] || \
      die "web bundle missing node_modules/$req — incomplete release artifact (custom server cannot start).
  Use a newer catcode-web-*.tar.gz built by current release-web.sh."
  done
  if have node; then
    if ! ( cd "$dir" && node --input-type=module -e 'await import("next")' >/dev/null 2>&1 ); then
      die "web bundle cannot resolve 'next' from $dir (module layout broken)"
    fi
  fi
  log_ok "Web bundle looks runnable ($dir)"
}

# True when an existing unit/plist looks like one written by this installer.
web_unit_is_installer_managed() {
  local unit="$1"
  [[ -f "$unit" ]] || return 1
  grep -q 'Managed-by: install.sh' "$unit" 2>/dev/null && return 0
  # Download path + source path both set CATCODE_CORE; custom run-web.sh units do not.
  grep -q 'Environment=CATCODE_CORE=' "$unit" 2>/dev/null
}

protect_existing_web_service() {
  $FORCE_WEB_SERVICE && return 0
  $SKIP_SERVICE && return 0
  if [[ "$PLATFORM" == "Darwin" ]]; then
    local plist="$HOME/Library/LaunchAgents/${LAUNCHD_LABEL}.plist"
    [[ -f "$plist" ]] || return 0
    if web_unit_is_installer_managed "$plist"; then return 0; fi
    die "Refusing to overwrite existing launchd agent $plist (not managed by install.sh).
  Keep your current web setup, or pass --force-web-service to replace it,
  or --skip-service to install files only."
  fi
  local unit="/etc/systemd/system/$UNIT_NAME"
  [[ -f "$unit" ]] || return 0
  if web_unit_is_installer_managed "$unit"; then return 0; fi
  die "Refusing to overwrite existing $unit (not managed by install.sh).
  Your current web service looks custom (e.g. scripts/run-web.sh). Options:
    • keep it: omit --with-web / --add-web
    • replace it: pass --force-web-service
    • files only: pass --skip-service (writes the web bundle, leaves the unit alone)"
}

# systemd unit (Linux) — runs the prebuilt standalone server (node start.js).
install_web_systemd_download() {
  protect_existing_web_service
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
$(systemd_env_lines)
ExecStart=$RT_BIN $WEB_DIR/start.js
Restart=on-failure
RestartSec=3
# NOTE: runs the prebuilt Next.js standalone server. For public exposure, put
# a reverse proxy (caddy/nginx) with TLS in front and bind --host 127.0.0.1.
# Managed-by: install.sh

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
  protect_existing_web_service
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
$(launchd_env_pairs)
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
    have node || have bun || missing+=("Node.js >= 22.13.0 or Bun (to RUN the web frontend — https://nodejs.org or https://bun.sh)")
  fi
  if [[ ${#missing[@]} -gt 0 ]]; then
    printf "\n  ${C_BOLD}${C_RED}Missing dependencies:${C_RST}\n" >&2
    local m
    for m in "${missing[@]}"; do printf "    ${C_RED}• %s${C_RST}\n" "$m" >&2; done
    die "install the dependencies above, then re-run."
  fi
  [[ "$SVC_MGR" != "unsupported" ]] \
    || die "install.sh supports Linux and macOS only (this is '$PLATFORM'). Windows users: see install.ps1 (-WithWeb for the web service)"
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

  # Bail before touching binaries when --with-web would clobber a custom unit.
  if $WITH_WEB && ! $SKIP_SERVICE; then
    protect_existing_web_service
  fi

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
    || die "install.sh supports Linux and macOS only (this is '$PLATFORM'). Windows users: see install.ps1 (-WithWeb for the web service)"
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
  detect_runtime build
  run_step --cwd "$REPO_DIR/sdk" "Installing SDK deps ($RT)" $RT install \
    || die "SDK dependency install failed"
  run_step --cwd "$REPO_DIR/sdk" "Building SDK (tsc)" $RT run build \
    || die "SDK build failed (sdk/dist/)"
  run_step --cwd "$REPO_DIR/web" "Installing web deps ($RT)" $RT install \
    || die "web dependency install failed"
  run_step --cwd "$REPO_DIR/web" "Building web (next build)" \
    env NEXT_TELEMETRY_DISABLED=1 CATCODE_VERSION_SOURCE=source $RT run build \
    || die "web build failed (next build)"
  # Stamp version.json into the web working dir (systemd WorkingDirectory).
  write_web_version_json "$REPO_DIR/web" "${VERSION_DETECTED:-unknown}" "source"
}

install_web_systemd_source() {
  protect_existing_web_service
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
$(systemd_env_lines)
ExecStart=$exec_start
Restart=on-failure
RestartSec=3
# Managed-by: install.sh

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
  protect_existing_web_service
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
$(launchd_env_pairs)
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
  if $SKIP_SERVICE; then
    log_info "Skipping web service install (--skip-service)"
    return 0
  fi
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
  # --skip-service is a files-only / side-install path; do not rewrite the
  # system installer.state (would clobber a real install's prefix/version).
  if $SKIP_SERVICE; then
    log_info "Skipping installer state write (--skip-service)"
    return 0
  fi
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
EXPOSE_MODE="$EXPOSE_MODE"
ORIGIN_OVERRIDE="${ORIGIN_OVERRIDE:-}"
ORIGIN="${ORIGIN:-}"
WORKSPACE="${WORKSPACE:-}"
TERM_SHELL="${TERM_SHELL:-}"
IDLE_GC_MS="${IDLE_GC_MS:-}"
INSTALLER_URL="${INSTALLER_URL:-}"
WIN_INSTALLER_URL="${WIN_INSTALLER_URL:-}"
TRUSTED_ORIGINS="${TRUSTED_ORIGINS:-}"
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
  finalize_web_env
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
  [[ "${WEB_INSTALLED:-no}" == yes ]] && WITH_WEB=true
  finalize_web_env
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

# ── add-web: install the web service onto an existing install ─
do_add_web() {
  phase "Reading previous install state"
  if ! load_state; then
    die "no previous install found at $STATE_FILE — run 'bash install.sh' first to install catcode."
  fi
  if [[ "${WEB_INSTALLED:-no}" == yes ]]; then
    log_warn "web service is already installed — reinstalling it"
  fi
  WITH_WEB=true
  finalize_web_env
  if ! $SKIP_SERVICE; then
    protect_existing_web_service
  fi
  if [[ "${METHOD:-download}" == "source" ]]; then
    do_add_web_source
  else
    do_add_web_download
  fi
  save_state
  summary_add_web
}

do_add_web_download() {
  phase "Checking dependencies"
  check_deps_download
  ensure_sudo
  phase "Resolving release (matching installed version ${VERSION:-latest})"
  detect_os_tag
  detect_arch
  VERSION_OVERRIDE="${VERSION:-}"
  resolve_release
  log_info "Version:  $VER (tag $TAG)"
  log_info "Prefix:   $PREFIX"
  # preserve a custom web dir recorded by the previous install
  [[ -n "${WEB_DIR:-}" ]] && WEB_DIR_OVERRIDE="$WEB_DIR"

  phase "Installing catcode-core (for the web service)"
  local core_asset="catcode-core-${VER}-${OS_TAG}-${ARCH}"
  fetch_asset "$core_asset"
  run_root "Creating $PREFIX" mkdir -p "$PREFIX"
  run_root "Installing catcode-core -> $PREFIX/catcode-core" install -m 0755 "$TMPDIR_SELF/$core_asset" "$PREFIX/catcode-core"

  phase "Installing web service (prebuilt)"
  install_web_download
}

do_add_web_source() {
  phase "Checking dependencies (source build)"
  check_deps_source
  ensure_sudo
  if [[ -n "${REPO_DIR:-}" && -d "$REPO_DIR" ]]; then
    cd "$REPO_DIR"
  else
    resolve_repo
  fi
  detect_version
  phase "Building Rust core (catcode-core)"
  build_core
  phase "Building web frontend (SDK + Next.js)"
  build_web_source
  phase "Installing catcode-core"
  install_bins_source
  phase "Installing web service"
  install_web_service_source
}

# ── reinstall: reinstall the currently-installed version ─────
do_reinstall() {
  phase "Reading previous install state"
  if ! load_state; then
    die "no previous install found at $STATE_FILE — run 'bash install.sh' first."
  fi
  VERSION_OVERRIDE="${VERSION:-}"
  [[ "${WEB_INSTALLED:-no}" == yes ]] && WITH_WEB=true
  log_info "Reinstalling version ${VERSION:-latest} (method: ${METHOD:-download}, web: ${WEB_INSTALLED:-no})"
  finalize_web_env
  if [[ "${METHOD:-download}" == "source" ]]; then
    do_reinstall_source
  else
    do_install_download
  fi
}

do_reinstall_source() {
  phase "Rebuilding from source (no git pull)"
  if [[ -n "${REPO_DIR:-}" && -d "$REPO_DIR" ]]; then
    cd "$REPO_DIR"
  else
    resolve_repo
  fi
  detect_version
  ensure_sudo
  phase "Rebuilding Rust core"
  build_core
  phase "Rebuilding Go TUI"
  build_tui
  phase "Reinstalling binaries"
  install_bins_source
  if [[ "${WEB_INSTALLED:-no}" == yes ]]; then
    WITH_WEB=true
    phase "Rebuilding web frontend"
    build_web_source
    phase "Restarting web service"
    restart_web_service_source
  fi
  save_state
  summary_install
}

# ── status: show the current install state ───────────────────
do_status() {
  phase "Install status"
  if ! load_state; then
    log_warn "no previous install found at $STATE_FILE"
    log_info "Catalyst Code does not appear to be installed (no state file)."
    return 0
  fi
  log_info "Version:      ${VERSION:-(unknown)}"
  log_info "Method:       ${METHOD:-download}"
  log_info "Prefix:       ${PREFIX:-/usr/local/bin}"
  log_info "Web service:  ${WEB_INSTALLED:-no}"
  if [[ "${WEB_INSTALLED:-no}" == yes ]]; then
    log_info "Web dir:      ${WEB_DIR:-(unknown)}"
    log_info "Web address:  http://${HOST:-0.0.0.0}:${PORT:-49283}"
    log_info "Expose:       ${EXPOSE_MODE:-(unknown)}"
    [[ -n "${ORIGIN:-}" ]] && log_info "Origin:       ${ORIGIN}"
  fi
  log_info "Installed at: ${INSTALLED_AT:-(unknown)}"
  if [[ -x "${PREFIX:-/usr/local/bin}/catcode" ]]; then
    log_ok "catcode present at ${PREFIX:-/usr/local/bin}/catcode"
  else
    log_warn "catcode NOT found at ${PREFIX:-/usr/local/bin}/catcode"
  fi
  if [[ "${WEB_INSTALLED:-no}" == yes ]]; then
    if [[ -x "${PREFIX:-/usr/local/bin}/catcode-core" ]]; then
      log_ok "catcode-core present at ${PREFIX:-/usr/local/bin}/catcode-core"
    else
      log_warn "catcode-core NOT found at ${PREFIX:-/usr/local/bin}/catcode-core"
    fi
  fi
}

# ── summaries ───────────────────────────────────────────────
summary_install() {
  local web_line="(not installed — run with --with-web)"
  local svc_line=""
  if $WITH_WEB; then
    local svc_id="$UNIT_NAME"
    [[ "$PLATFORM" == "Darwin" ]] && svc_id="$LAUNCHD_LABEL (launchd)"
    if $SKIP_SERVICE; then
      web_line="${WEB_DIR:-<web-dir>}  (files only — service not started)"
      svc_line="service:   skipped (--skip-service)"
    else
      web_line="http://${HOST}:${PORT}  (running as $svc_id)"
      svc_line="service:   $svc_id  (enabled, auto-restart)"
    fi
  fi
  local expose_line=""
  $WITH_WEB && expose_line="expose:    ${EXPOSE_MODE}  origin: ${ORIGIN:-<auto>}"
  print_box "✓  Installed  ${APP_NAME}  v${VERSION_DETECTED}" \
    "tui:       $PREFIX/catcode" \
    "core:      $PREFIX/catcode-core" \
    "web:       $web_line" \
    "$expose_line" \
    "$svc_line" \

    "update:    catcode --update  (or bash install.sh --update)" \
    "uninstall: bash install.sh --uninstall" \
    "log:       ${LOG_FILE:-<disabled>}"
  log_info "Run the TUI with:  catcode"
  if $WITH_WEB && ! $SKIP_SERVICE; then
    if [[ "$PLATFORM" == "Darwin" ]]; then
      log_info "Web service logs:  tail -f $HOME/Library/Logs/catalyst-code-web.log"
    else
      log_info "Web service logs:  journalctl -u $UNIT_NAME -f"
    fi
    log_warn "Auth: on first launch use /login to paste an API key or complete OAuth — nothing is auto-detected."
    if [[ "$HOST" != "127.0.0.1" ]]; then
      log_warn "Bound to $HOST — put a TLS reverse proxy in front for public use."
    fi
  elif $WITH_WEB && $SKIP_SERVICE; then
    log_info "Start the web manually:  cd ${WEB_DIR:-<web-dir>} && PORT=$PORT HOSTNAME=$HOST CATCODE_CORE=$PREFIX/catcode-core node start.js"
  fi
}

summary_update() {
  local web_line="(web service not installed)"
  [[ "${WEB_INSTALLED:-no}" == yes ]] && web_line="http://${HOST}:${PORT}  (restarted)"
  local expose_line=""
  [[ "${WEB_INSTALLED:-no}" == yes ]] && expose_line="expose: ${EXPOSE_MODE}  origin: ${ORIGIN:-<auto>}"
  print_box "✓  Updated  ${APP_NAME}  v${VERSION_DETECTED}" \
    "tui:    $PREFIX/catcode" \
    "core:   $PREFIX/catcode-core" \
    "web:    $web_line" \
    "$expose_line" \
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

summary_add_web() {
  local svc_id="$UNIT_NAME"
  [[ "$PLATFORM" == "Darwin" ]] && svc_id="$LAUNCHD_LABEL (launchd)"
  print_box "✓  Web service added  ${APP_NAME}  v${VERSION_DETECTED}" \
    "core:      $PREFIX/catcode-core" \
    "web:       http://${HOST}:${PORT}  (running as $svc_id)" \
    "expose:    ${EXPOSE_MODE}  origin: ${ORIGIN:-<auto>}" \
    "service:   $svc_id  (enabled, auto-restart)" \
    "update:    catcode --update  (or bash install.sh --update)" \
    "uninstall: bash install.sh --uninstall"
  if [[ "$PLATFORM" == "Darwin" ]]; then
    log_info "Web service logs:  tail -f $HOME/Library/Logs/catalyst-code-web.log"
  else
    log_info "Web service logs:  journalctl -u $UNIT_NAME -f"
  fi
  log_warn "Auth: on first launch use /login to paste an API key or complete OAuth — nothing is auto-detected."
}

# ── interactive install settings (menu path only) ─────────────
# Prompt for common knobs; Enter keeps the shown default. Only runs on a
# real TTY after the menu — flagged installs (curl|bash, CI) skip this.
prompt_value() {
  local label="$1" default="$2" ans
  if [[ -n "$default" ]]; then
    read -rp "  ${C_CYAN}${label}${C_RST} [${C_DIM}${default}${C_RST}]: " ans || true
  else
    read -rp "  ${C_CYAN}${label}${C_RST} [${C_DIM}empty / latest${C_RST}]: " ans || true
  fi
  if [[ -z "${ans:-}" ]]; then
    printf '%s' "$default"
  else
    printf '%s' "$ans"
  fi
}

default_web_dir_prompt() {
  if [[ -n "$WEB_DIR_OVERRIDE" ]]; then
    printf '%s' "$WEB_DIR_OVERRIDE"
  elif [[ "$PLATFORM" == "Darwin" ]]; then
    printf '%s' "$HOME/Library/Application Support/catalyst-code/web"
  else
    printf '%s' "/opt/catalyst-code/web"
  fi
}

prompt_install_options() {
  [[ -t 0 ]] || return 0
  case "$ACTION" in
    install|add-web|update|reinstall) ;;
    *) return 0 ;;
  esac

  local customize=""
  printf "\n"
  read -rp "  Customize install settings (paths, port, version)? [y/N]: " customize || true
  case "${customize:-}" in
    y|Y|yes|YES) ;;
    *)
      log_info "Using defaults (prefix=$PREFIX port=$PORT expose=$EXPOSE_MODE)"
      return 0
      ;;
  esac

  printf "\n  ${C_BOLD}Install settings${C_RST}  ${C_DIM}(press Enter to keep each default)${C_RST}\n\n"

  local v
  v="$(prompt_value "Binary install directory" "$PREFIX")"
  PREFIX="$v"

  v="$(prompt_value "Release version pin" "${VERSION_OVERRIDE}")"
  VERSION_OVERRIDE="$v"

  v="$(prompt_value "Download base URL (mirror)" "${BASE_URL_OVERRIDE}")"
  BASE_URL_OVERRIDE="$v"

  local want_web=false
  $WITH_WEB && want_web=true
  [[ "$ACTION" == "add-web" ]] && want_web=true
  if ! $want_web && [[ "$ACTION" == "update" || "$ACTION" == "reinstall" ]]; then
    if [[ -f "$STATE_FILE" ]] && grep -Eq '^WEB_INSTALLED="yes"' "$STATE_FILE" 2>/dev/null; then
      want_web=true
    fi
  fi

  if $want_web; then
    v="$(prompt_value "Web bundle directory" "$(default_web_dir_prompt)")"
    WEB_DIR_OVERRIDE="$v"

    while true; do
      v="$(prompt_value "Web service port" "$PORT")"
      if [[ "$v" =~ ^[0-9]+$ ]] && ((10#$v >= 1 && 10#$v <= 65535)); then
        PORT="$v"; WANT_PORT="$v"
        break
      fi
      printf "  ${C_YELLOW}port must be an integer 1–65535${C_RST}\n"
    done

    # Expose mode + canonical origin (drives bind host + CATCODE_WEB_ORIGIN).
    v="$(prompt_value "Expose mode (local|intranet|public)" "${EXPOSE_MODE}")"
    case "$v" in
      local|intranet|public) EXPOSE_MODE="$v"; WANT_EXPOSE="$v" ;;
      *) log_warn "invalid expose mode '$v' — keeping ${EXPOSE_MODE}" ;;
    esac
    if [[ "$EXPOSE_MODE" == "public" ]]; then
      v="$(prompt_value "Public origin URL (https://…)" "${ORIGIN_OVERRIDE}")"
      [[ -n "$v" ]] && ORIGIN_OVERRIDE="$v"
    elif [[ "$EXPOSE_MODE" == "intranet" ]]; then
      local lan_ip
      if lan_ip="$(detect_lan_ip 2>/dev/null)"; then
        log_info "Detected LAN IP: $lan_ip (origin → http://$lan_ip:$PORT)"
      else
        log_warn "no LAN IP detected — set the origin override below for non-loopback access"
      fi
      v="$(prompt_value "Origin URL override (Enter = auto-detect)" "${ORIGIN_OVERRIDE}")"
      [[ -n "$v" ]] && ORIGIN_OVERRIDE="$v"
    fi

    # Advanced web-service envs (optional).
    local advanced=""
    read -rp "  ${C_CYAN}Show advanced web settings${C_RST} ${C_DIM}(workspace, shell, idle-gc, installer URLs)?${C_RST} [y/N]: " advanced || true
    case "${advanced:-}" in
      y|Y|yes|YES)
        v="$(prompt_value "Default workspace (core opens here)" "${WORKSPACE}")"; WORKSPACE="$v"; WANT_WORKSPACE="$v"
        v="$(prompt_value "Terminal shell (e.g. /bin/bash, /bin/zsh)" "${TERM_SHELL}")"; TERM_SHELL="$v"; WANT_SHELL="$v"
        v="$(prompt_value "Idle session GC ms (0=disable)" "${IDLE_GC_MS:-$DEFAULT_IDLE_GC_MS}")"; IDLE_GC_MS="$v"; WANT_IDLE_GC="$v"
        v="$(prompt_value "Self-update installer URL (install.sh)" "${INSTALLER_URL}")"; INSTALLER_URL="$v"; WANT_INSTALLER_URL="$v"
        v="$(prompt_value "Self-update Windows installer URL (install.ps1)" "${WIN_INSTALLER_URL}")"; WIN_INSTALLER_URL="$v"; WANT_WIN_INSTALLER_URL="$v"
        v="$(prompt_value "Additional trusted origins (comma-sep, for proxy/multi-domain)" "${TRUSTED_ORIGINS}")"; TRUSTED_ORIGINS="$v"; WANT_TRUSTED_ORIGINS="$v"
        ;;
    esac
  fi

  printf "\n"
  log_ok "Will use: prefix=$PREFIX  port=$PORT  expose=$EXPOSE_MODE"
  [[ -n "$ORIGIN_OVERRIDE" ]] && log_ok "origin: $ORIGIN_OVERRIDE"
  [[ -n "$VERSION_OVERRIDE" ]] && log_ok "version pin: $VERSION_OVERRIDE"
  [[ -n "$BASE_URL_OVERRIDE" ]] && log_ok "base URL: $BASE_URL_OVERRIDE"
  [[ -n "$WEB_DIR_OVERRIDE" ]] && log_ok "web dir: $WEB_DIR_OVERRIDE"
  [[ -n "$WORKSPACE" ]] && log_ok "workspace: $WORKSPACE"
}

# ── interactive menu (no args + a terminal) ──────────────────
show_menu() {
  local v=""
  [[ -f "$STATE_FILE" ]] && v="$(grep -E '^VERSION=' "$STATE_FILE" 2>/dev/null | head -1 | sed -E 's/.*="([^"]*)".*/\1/')"
  local status_line="not installed"
  [[ -n "$v" ]] && status_line="installed v${v}"
  print_box "Catalyst Code — installer menu" \
    "platform: ${PLATFORM} (${SVC_MGR})" \
    "status:   ${status_line}"
  local choice
  while true; do
    cat <<EOF
  ${C_DIM}What would you like to do?${C_RST}

    ${C_BOLD}1${C_RST}  Install              ${C_DIM}catcode TUI + core${C_RST}
    ${C_BOLD}2${C_RST}  Install with web     ${C_DIM}TUI + core + 24/7 web service${C_RST}
    ${C_BOLD}3${C_RST}  Add web service      ${C_DIM}add web to an existing install${C_RST}
    ${C_BOLD}4${C_RST}  Update               ${C_DIM}download latest + reinstall${C_RST}
    ${C_BOLD}5${C_RST}  Reinstall            ${C_DIM}reinstall the current version${C_RST}
    ${C_BOLD}6${C_RST}  Uninstall            ${C_DIM}remove everything${C_RST}
    ${C_BOLD}7${C_RST}  Status               ${C_DIM}show current install state${C_RST}
    ${C_BOLD}0${C_RST}  Exit

EOF
    read -rp "  ${C_CYAN}Select [0-7]:${C_RST} " choice || { printf "\n  ${C_DIM}Bye.${C_RST}\n"; exit 0; }
    case "$choice" in
      1) ACTION="install"; break ;;
      2) ACTION="install"; WITH_WEB=true; break ;;
      3) ACTION="add-web"; break ;;
      4) ACTION="update"; break ;;
      5) ACTION="reinstall"; break ;;
      6) ACTION="uninstall"; break ;;
      7) ACTION="status"; break ;;
      0) printf "  ${C_DIM}Bye.${C_RST}\n"; exit 0 ;;
      *) printf "  ${C_YELLOW}invalid choice — try again${C_RST}\n" ;;
    esac
  done
}

# ── main ────────────────────────────────────────────────────
main() {
  # No arguments + a real terminal → interactive menu. Anything else (flags,
  # or a non-TTY stdin such as `curl … | bash`) runs the action directly.
  if [[ $# -eq 0 && -t 0 ]]; then
    show_menu
    prompt_install_options
  else
    parse_args "$@"
  fi
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
    add-web)   do_add_web ;;
    reinstall) do_reinstall ;;
    status)    do_status ;;
    *)         die "unknown action: $ACTION" ;;
  esac
}

main "$@"
