package main

// Self-update for the catcode TUI.
//
// The harness releases are keyed by git commit short SHA (each push to master
// produces a GitHub Release named/tagged after the commit). A released binary
// carries its version in `coreVersion` (injected via -ldflags at build). So an
// "update" = the latest release's tag differs from coreVersion.
//
// Two surfaces:
//   1. On launch, a background check (cache-backed, 6h TTL) fetches the latest
//      release tag and, if it differs from coreVersion, sends updateAvailableMsg
//      so the TUI renders a one-line "update available" banner.
//   2. `catcode --update` downloads the matching platform asset, verifies its
//      sha256 sidecar, and atomically replaces the running executable.
//
// Both tolerate being offline: the launch check is silent on failure; --update
// prints a clear error and exits non-zero.

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

const (
	githubRepo       = "catalystctl/catcode"
	updateCacheTTL   = 6 * time.Hour
	updateAPITimeout = 12 * time.Second
	updateDLOTimeout = 10 * time.Minute
)

// ghAsset is a single release asset in the GitHub API response.
type ghAsset struct {
	Name               string `json:"name"`
	BrowserDownloadURL string `json:"browser_download_url"`
}

// ghRelease is the subset of /releases/latest we need.
type ghRelease struct {
	TagName string    `json:"tag_name"`
	Name    string    `json:"name"`
	Assets  []ghAsset `json:"assets"`
}

// updateInfo describes an available update.
type updateInfo struct {
	current string
	latest  string
}

// updateAvailableMsg is sent (once) by the launch-time check when a newer
// release is available. The session stores it and renders a banner.
type updateAvailableMsg struct {
	info updateInfo
}

// -----------------------------------------------------------------------------
// Platform → asset-name mapping (must match release-{linux,macos,windows}.sh)
// -----------------------------------------------------------------------------

// osTag maps runtime.GOOS to the release-asset label (darwin -> macos).
func osTag() string {
	if runtime.GOOS == "darwin" {
		return "macos"
	}
	return runtime.GOOS
}

// archTag maps runtime.GOARCH to the release-asset label (amd64 -> x86_64).
func archTag() string {
	if runtime.GOARCH == "amd64" {
		return "x86_64"
	}
	return runtime.GOARCH
}

// assetName is the release-asset filename for the running platform, e.g.
// "catcode-1a0228e-linux-x86_64" or "catcode-1a0228e-windows-x86_64.exe".
func assetName(ver string) string {
	return fmt.Sprintf("catcode-%s-%s-%s%s", ver, osTag(), archTag(), coreExeSuffix())
}

// -----------------------------------------------------------------------------
// GitHub API
// -----------------------------------------------------------------------------

// newGHRequest builds an authenticated-ish GET to the GitHub API/releases.
func newGHRequest(url string) (*http.Request, error) {
	req, err := http.NewRequest(http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Accept", "application/vnd.github+json")
	req.Header.Set("User-Agent", "catcode/"+coreVersion+" (self-updater)")
	return req, nil
}

// fetchLatestRelease returns the latest release from GitHub, or an error.
func fetchLatestRelease() (*ghRelease, error) {
	url := "https://api.github.com/repos/" + githubRepo + "/releases/latest"
	req, err := newGHRequest(url)
	if err != nil {
		return nil, err
	}
	cl := &http.Client{Timeout: updateAPITimeout}
	resp, err := cl.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusNotFound {
		return nil, fmt.Errorf("no releases published for %s yet", githubRepo)
	}
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("github api returned HTTP %d", resp.StatusCode)
	}
	var rel ghRelease
	if err := json.NewDecoder(resp.Body).Decode(&rel); err != nil {
		return nil, err
	}
	return &rel, nil
}

// compareUpdate returns a non-nil updateInfo when `latest` is a real update
// (different from the running version, and we're not a dev build). nil means
// "up to date" (or dev build — never nag).
func compareUpdate(latest string) *updateInfo {
	if coreVersion == "dev" || latest == "" || latest == coreVersion {
		return nil
	}
	return &updateInfo{current: coreVersion, latest: latest}
}

// -----------------------------------------------------------------------------
// Launch-time check (cache-backed, async, silent on failure)
// -----------------------------------------------------------------------------

// updateCacheFile lives beside the embedded-core extraction cache dir.
func updateCacheFile() (string, bool) {
	dir, err := os.UserCacheDir()
	if err != nil || dir == "" {
		home, hErr := os.UserHomeDir()
		if hErr != nil || home == "" {
			return "", false
		}
		dir = filepath.Join(home, ".cache")
	}
	return filepath.Join(dir, "catalyst-code", "update-check.json"), true
}

type updateCache struct {
	CheckedAt int64  `json:"checked_at"`
	Latest    string `json:"latest"`
}

// readUpdateCache returns the cached latest tag if the cache is fresh
// (< updateCacheTTL), else ("", false) → caller should fetch.
func readUpdateCache() (string, bool) {
	p, ok := updateCacheFile()
	if !ok {
		return "", false
	}
	b, err := os.ReadFile(p)
	if err != nil {
		return "", false
	}
	var c updateCache
	if err := json.Unmarshal(b, &c); err != nil || c.Latest == "" {
		return "", false
	}
	if time.Since(time.Unix(c.CheckedAt, 0)) > updateCacheTTL {
		return "", false
	}
	return c.Latest, true
}

// writeUpdateCache persists a freshly fetched latest tag.
func writeUpdateCache(latest string) {
	p, ok := updateCacheFile()
	if !ok {
		return
	}
	_ = os.MkdirAll(filepath.Dir(p), 0o755)
	c := updateCache{CheckedAt: time.Now().Unix(), Latest: latest}
	b, err := json.Marshal(c)
	if err != nil {
		return
	}
	// best-effort atomic write (temp + rename in the same dir)
	tmp, err := os.CreateTemp(filepath.Dir(p), ".update-check.*.tmp")
	if err != nil {
		return
	}
	tmpName := tmp.Name()
	if _, err := tmp.Write(b); err != nil {
		tmp.Close()
		os.Remove(tmpName)
		return
	}
	tmp.Close()
	if err := os.Rename(tmpName, p); err != nil {
		os.Remove(tmpName)
	}
}

// launchUpdateCheck is called from main() right after the program is created.
// It never blocks the UI: a fresh cache answers instantly; a stale/missing
// cache triggers an async fetch that sends updateAvailableMsg on success.
func launchUpdateCheck(prog *tea.Program) {
	if coreVersion == "dev" {
		return // never nag a dev build
	}
	// 1) fresh cache → answer immediately, no network.
	if latest, ok := readUpdateCache(); ok {
		if info := compareUpdate(latest); info != nil {
			// Send from a goroutine: prog.Send blocks on a select until the
			// event loop (started by prog.Run(), called after this returns)
			// is draining messages. Calling it synchronously here deadlocks
			// the main goroutine before Run() ever starts — the runtime then
			// detects "all goroutines are asleep" and panics. This only
			// triggers when a fresh cache reports an available update, hence
			// the intermittent "after updating/installing" reports.
			go prog.Send(updateAvailableMsg{info: *info})
		}
		return
	}
	// 2) stale/missing → fetch async, cache, and notify.
	go func() {
		rel, err := fetchLatestRelease()
		if err != nil || rel == nil || rel.TagName == "" {
			return // silent on failure (offline, rate-limited, …)
		}
		writeUpdateCache(rel.TagName)
		if info := compareUpdate(rel.TagName); info != nil {
			prog.Send(updateAvailableMsg{info: *info})
		}
	}()
}

// -----------------------------------------------------------------------------
// `catcode --update` (and --check-update / --version / --help)
// -----------------------------------------------------------------------------

// handleCLIArgs inspects os.Args before the TUI starts. If a recognized flag is
// present it performs the action and returns (exitCode, true); otherwise
// (0, false) so main() proceeds to launch the TUI.
func handleCLIArgs(args []string) (int, bool) {
	for _, a := range args {
		switch a {
		case "-h", "--help":
			printUsage()
			return 0, true
		case "-v", "--version":
			fmt.Printf("catcode %s\n", coreVersion)
			fmt.Printf("repo: github.com/%s\n", githubRepo)
			return 0, true
		case "--check-update":
			return runCheckUpdate(), true
		case "--update", "-u", "update":
			return runUpdate(), true
		}
	}
	return 0, false
}

func printUsage() {
	fmt.Print(`catcode — Catalyst Code TUI

Usage:
  catcode                       start the interactive TUI
  catcode --update              update to the latest GitHub release
  catcode --check-update        report whether an update is available
  catcode --version, -v         print the current version
  catcode --help, -h            show this help

The version is the git commit short SHA of the build. Run ` + "`catcode --update`" + `
to fetch the matching platform binary from github.com/` + githubRepo + ` and
atomically replace this executable.
`)
}

// runCheckUpdate prints whether an update is available (network). Exits 0
// regardless of the answer so it's scripting-friendly.
func runCheckUpdate() int {
	rel, err := fetchLatestRelease()
	if err != nil {
		fmt.Fprintf(os.Stderr, "catcode: could not check for updates: %v\n", err)
		return 1
	}
	switch {
	case coreVersion == "dev":
		fmt.Printf("catcode (dev build) — latest release: %s\n", rel.TagName)
		fmt.Println("Run `catcode --update` to install it.")
	case rel.TagName == coreVersion:
		fmt.Printf("catcode %s — up to date\n", coreVersion)
	default:
		fmt.Printf("Update available: %s  (you're on %s)\n", rel.TagName, coreVersion)
		fmt.Println("Run `catcode --update` to install it.")
	}
	return 0
}

// findAsset returns the release asset whose name matches assetName(ver).
func findAsset(rel *ghRelease, name string) *ghAsset {
	for i := range rel.Assets {
		if rel.Assets[i].Name == name {
			return &rel.Assets[i]
		}
	}
	return nil
}

// hashFile returns the SHA-256 hex of the file at p. (Distinct name from the
// embed_core-gated sha256File so it's available in every build.)
func hashFile(p string) (string, error) {
	f, err := os.Open(p)
	if err != nil {
		return "", err
	}
	defer f.Close()
	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return "", err
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}

// fetchSHA256 downloads <asset>.sha256 and returns the hex digest (first token).
func fetchSHA256(asset *ghAsset) (string, error) {
	req, err := newGHRequest(asset.BrowserDownloadURL + ".sha256")
	if err != nil {
		return "", err
	}
	cl := &http.Client{Timeout: updateAPITimeout}
	resp, err := cl.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("sha256 asset returned HTTP %d", resp.StatusCode)
	}
	b, err := io.ReadAll(io.LimitReader(resp.Body, 4096))
	if err != nil {
		return "", err
	}
	fields := strings.Fields(strings.TrimSpace(string(b)))
	if len(fields) == 0 {
		return "", fmt.Errorf("empty sha256 file")
	}
	return fields[0], nil
}

// download streams url to dst, printing a single-line progress meter to stderr.
func download(url, label string, dst *os.File) error {
	req, err := newGHRequest(url)
	if err != nil {
		return err
	}
	cl := &http.Client{Timeout: updateDLOTimeout}
	resp, err := cl.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("download returned HTTP %d", resp.StatusCode)
	}
	total := resp.ContentLength // -1 if unknown
	var written int64
	buf := make([]byte, 64*1024)
	last := time.Now()
	for {
		n, rerr := resp.Body.Read(buf)
		if n > 0 {
			if _, werr := dst.Write(buf[:n]); werr != nil {
				return werr
			}
			written += int64(n)
			if time.Since(last) > 100*time.Millisecond || (total > 0 && written == total) {
				last = time.Now()
				if total > 0 {
					fmt.Fprintf(os.Stderr, "\r  ↓ %s  %3d%%  %s   ", label, written*100/total, humanBytes(written))
				} else {
					fmt.Fprintf(os.Stderr, "\r  ↓ %s  %s   ", label, humanBytes(written))
				}
			}
		}
		if rerr == io.EOF {
			break
		}
		if rerr != nil {
			return rerr
		}
	}
	fmt.Fprintln(os.Stderr) // finish the progress line
	return nil
}

func humanBytes(n int64) string {
	const k = 1024
	switch {
	case n < k:
		return fmt.Sprintf("%dB", n)
	case n < k*k:
		return fmt.Sprintf("%.1fKB", float64(n)/k)
	case n < k*k*k:
		return fmt.Sprintf("%.1fMB", float64(n)/(k*k))
	default:
		return fmt.Sprintf("%.1fGB", float64(n)/(k*k*k))
	}
}

// selfReplace atomically swaps the freshly downloaded tmp file over the
// running executable. On Windows the running exe can't be overwritten in place,
// so it's moved aside to <exe>.old first (left for the next launch to delete).
func selfReplace(tmp, exe string) error {
	if runtime.GOOS == "windows" {
		old := exe + ".old"
		_ = os.Remove(old) // clean up a previous run's leftover (best-effort)
		if err := os.Rename(exe, old); err != nil {
			return err
		}
		if err := os.Rename(tmp, exe); err != nil {
			_ = os.Rename(old, exe) // try to restore the old binary
			return err
		}
		return nil
	}
	return os.Rename(tmp, exe)
}

// canWriteDir reports whether the current user can create (and remove) a
// file in dir. A system-wide install (e.g. /usr/local/bin, root-owned) is
// not writable by an unprivileged user — detecting this up front lets us
// escalate or fail with a clear message instead of crashing mid-download.
func canWriteDir(dir string) bool {
	f, err := os.CreateTemp(dir, ".catcode-probe.*")
	if err != nil {
		return false
	}
	f.Close()
	os.Remove(f.Name())
	return true
}

// tryEscalateSudo re-execs the current binary under sudo, passing through the
// original CLI args so the privileged child performs the real update. Returns
// (exitCode, true) if escalation was attempted (success or failure), or
// (0, false) if sudo isn't available on PATH.
func tryEscalateSudo(exe string) (int, bool) {
	sudo, err := exec.LookPath("sudo")
	if err != nil {
		return 0, false // no sudo (non-Unix, or not installed)
	}
	cmd := exec.Command(sudo, append([]string{exe}, os.Args[1:]...)...)
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		// Propagate the child's exit code when we can (e.g. user cancelled at
		// the sudo prompt, or the update itself failed); otherwise generic.
		if ee, ok := err.(*exec.ExitError); ok {
			if code := ee.ExitCode(); code >= 0 {
				return code, true
			}
		}
		fmt.Fprintf(os.Stderr, "  ✗ sudo escalation failed: %v\n", err)
		return 1, true
	}
	return 0, true
}

// runUpdate performs the full self-update and returns an exit code.
func runUpdate() int {
	exe, err := os.Executable()
	if err != nil {
		fmt.Fprintf(os.Stderr, "catcode: cannot resolve my own path: %v\n", err)
		return 1
	}
	if rp, e := filepath.EvalSymlinks(exe); e == nil {
		exe = rp // replace the real target, not a symlink
	}
	dir := filepath.Dir(exe)

	// A system-wide install (e.g. /usr/local/bin, root-owned) can't be written
	// to by an unprivileged user. Detect this before touching the network: if
	// we can't write to the install dir, either re-exec under sudo so the
	// update just works, or print a clear, actionable error — instead of
	// failing mid-download with a cryptic temp-file permission error.
	if !canWriteDir(dir) {
		if os.Geteuid() != 0 {
			fmt.Printf("catcode is installed system-wide (%s) and needs elevated privileges to update.\n", dir)
			fmt.Println("Re-running with sudo…")
			if code, ok := tryEscalateSudo(exe); ok {
				return code
			}
		}
		fmt.Fprintf(os.Stderr, "  ✗ permission denied: cannot write to %s\n", dir)
		fmt.Fprintf(os.Stderr, "    catcode is installed system-wide. Re-run with:\n")
		fmt.Fprintf(os.Stderr, "      sudo catcode --update\n")
		fmt.Fprintf(os.Stderr, "    or use the installer:\n")
		fmt.Fprintf(os.Stderr, "      bash install.sh --update\n")
		return 1
	}

	fmt.Println("Checking for updates…")
	rel, err := fetchLatestRelease()
	if err != nil {
		fmt.Fprintf(os.Stderr, "  ✗ could not fetch latest release: %v\n", err)
		return 1
	}

	if coreVersion != "dev" && rel.TagName == coreVersion {
		fmt.Printf("Already up to date (%s).\n", coreVersion)
		return 0
	}
	if coreVersion == "dev" {
		fmt.Printf("Updating dev build → %s\n", rel.TagName)
	} else {
		fmt.Printf("Updating %s → %s\n", coreVersion, rel.TagName)
	}

	name := assetName(rel.TagName)
	asset := findAsset(rel, name)
	if asset == nil {
		fmt.Fprintf(os.Stderr, "  ✗ no release asset for %s/%s (looking for %s)\n", osTag(), archTag(), name)
		avail := make([]string, 0, len(rel.Assets))
		for _, a := range rel.Assets {
			avail = append(avail, a.Name)
		}
		fmt.Fprintf(os.Stderr, "    available assets: %s\n", strings.Join(avail, ", "))
		return 1
	}

	// Download into a temp file in the SAME directory (so the rename is atomic
	// and never crosses a filesystem boundary).
	tmp, err := os.CreateTemp(dir, ".catcode-update.*"+coreExeSuffix())
	if err != nil {
		fmt.Fprintf(os.Stderr, "  ✗ could not create temp file: %v\n", err)
		return 1
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName) // cleanup on every exit path (no-op after a successful rename)

	if err := download(asset.BrowserDownloadURL, name, tmp); err != nil {
		tmp.Close()
		fmt.Fprintf(os.Stderr, "  ✗ download failed: %v\n", err)
		return 1
	}
	if err := tmp.Close(); err != nil {
		fmt.Fprintf(os.Stderr, "  ✗ could not finalize download: %v\n", err)
		return 1
	}

	// Verify integrity against the published sha256 sidecar.
	if want, err := fetchSHA256(asset); err == nil {
		got, gerr := hashFile(tmpName)
		if gerr != nil {
			fmt.Fprintf(os.Stderr, "  ✗ could not hash download: %v\n", gerr)
			return 1
		}
		if !strings.EqualFold(want, got) {
			fmt.Fprintf(os.Stderr, "  ✗ checksum mismatch (expected %s, got %s)\n", want, got)
			return 1
		}
		fmt.Println("  ✓ verified (sha256)")
	} else {
		fmt.Fprintf(os.Stderr, "  ! could not verify checksum (%v) — proceeding without verification\n", err)
	}

	// Match the existing file's permissions.
	mode := os.FileMode(0o755)
	if fi, err := os.Stat(exe); err == nil {
		mode = fi.Mode()
	}
	if err := os.Chmod(tmpName, mode); err != nil {
		fmt.Fprintf(os.Stderr, "  ✗ could not set permissions: %v\n", err)
		return 1
	}

	if err := selfReplace(tmpName, exe); err != nil {
		if os.IsPermission(err) {
			fmt.Fprintf(os.Stderr, "  ✗ permission denied writing to %s\n", exe)
			fmt.Fprintf(os.Stderr, "    re-run with elevated privileges, or use:\n")
			fmt.Fprintf(os.Stderr, "      bash install.sh --update\n")
		} else {
			fmt.Fprintf(os.Stderr, "  ✗ could not replace %s: %v\n", exe, err)
		}
		return 1
	}

	fmt.Printf("  ✓ updated catcode → %s\n", rel.TagName)
	fmt.Println("Relaunch catcode to use the new version.")
	return 0
}

// -----------------------------------------------------------------------------
// TUI banner (rendered inside the alt-screen when an update is available)
// -----------------------------------------------------------------------------

// renderUpdateBanner renders the one-line "update available" notice shown at the
// top of the TUI (right under the header separator) when s.updateInfo is set.
func (s *session) renderUpdateBanner() string {
	if s.updateInfo == nil {
		return ""
	}
	u := s.updateInfo
	head := warnStyle.Render("⚡ Update available")
	detail := dimStyle.Render(fmt.Sprintf(": %s  (you're on %s)  —  run ", u.latest, u.current))
	cmd := accentStyle.Render("catcode --update")
	line := head + detail + cmd

	// Clamp to the terminal width (truncate the middle, never wrap).
	if w := s.width; w > 0 && lipgloss.Width(line) > w {
		// Drop the "(you're on …)" clause before crude truncation; if still too
		// wide, hard-truncate with an ellipsis.
		detail = dimStyle.Render(fmt.Sprintf(": %s  —  run ", u.latest))
		line = head + detail + cmd
		if lipgloss.Width(line) > w {
			line = lipgloss.NewStyle().MaxWidth(w).Render(line)
		}
	}
	return line
}
