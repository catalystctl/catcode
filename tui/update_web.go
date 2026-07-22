package main

// Web + core companion updates for `catcode --update`.
//
// When the installer (or a recognizable web install dir) is present,
// `runUpdate` also refreshes:
//   - catcode-core (service binary next to catcode / Windows Programs dir)
//   - catcode-web-<ver>.tar.gz extracted into WEB_DIR
//   - service restart (systemd / launchd / NSSM / Scheduled Task)
//
// Detection order (all OS):
//   1. Installer state file(s) with WEB_INSTALLED=yes + WEB_DIR
//   2. Well-known per-OS web dirs with start.js + version.json/server.js

import (
	"archive/tar"
	"compress/gzip"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
)

const (
	unixInstallerStatePath = "/etc/catalyst-code/installer.state"
	windowsSvcName         = "CatalystCodeWeb"
	windowsTaskName        = "CatalystCodeWeb"
)

// installerState is the shell-sourcable file written by install.sh /
// packaging/windows/install-web.ps1.
type installerState struct {
	Method       string
	Prefix       string
	WebDir       string
	WebInstalled bool
	UnitName     string
	Version      string
}

// installerStatePaths returns candidate state files for this OS.
func installerStatePaths() []string {
	var paths []string
	switch runtime.GOOS {
	case "windows":
		if local := os.Getenv("LOCALAPPDATA"); local != "" {
			paths = append(paths, filepath.Join(local, "catalyst-code", "installer.state"))
		}
		if home, err := os.UserHomeDir(); err == nil && home != "" {
			paths = append(paths, filepath.Join(home, "AppData", "Local", "catalyst-code", "installer.state"))
		}
	default:
		paths = append(paths, unixInstallerStatePath)
		if home, err := os.UserHomeDir(); err == nil && home != "" {
			paths = append(paths, filepath.Join(home, ".config", "catalyst-code", "installer.state"))
		}
	}
	return paths
}

// parseInstallerStateFile parses one shell-sourcable installer.state.
func parseInstallerStateFile(path string) (*installerState, bool) {
	b, err := os.ReadFile(path)
	if err != nil {
		return nil, false
	}
	st := &installerState{
		Prefix:   defaultInstallPrefix(),
		UnitName: defaultWebServiceName(),
	}
	for _, line := range strings.Split(string(b), "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		key, val, ok := strings.Cut(line, "=")
		if !ok {
			continue
		}
		val = strings.Trim(val, `"'`)
		switch key {
		case "METHOD":
			st.Method = val
		case "PREFIX":
			if val != "" {
				st.Prefix = val
			}
		case "WEB_DIR":
			st.WebDir = val
		case "WEB_INSTALLED":
			st.WebInstalled = val == "yes" || val == "true" || val == "1"
		case "UNIT_NAME", "SVC_NAME", "SERVICE_NAME":
			if val != "" {
				st.UnitName = val
			}
		case "VERSION":
			st.Version = val
		}
	}
	return st, true
}

// loadInstallerState returns the first readable installer state on this machine.
func loadInstallerState() (*installerState, bool) {
	for _, p := range installerStatePaths() {
		if st, ok := parseInstallerStateFile(p); ok {
			return st, true
		}
	}
	return nil, false
}

func defaultInstallPrefix() string {
	if runtime.GOOS == "windows" {
		if local := os.Getenv("LOCALAPPDATA"); local != "" {
			return filepath.Join(local, "Programs", "catcode")
		}
		return "."
	}
	return "/usr/local/bin"
}

func defaultWebServiceName() string {
	if runtime.GOOS == "windows" {
		return windowsSvcName
	}
	if runtime.GOOS == "darwin" {
		return "com.catalyst-code.web"
	}
	return "catalyst-code-web.service"
}

func defaultWebDirs() []string {
	var dirs []string
	switch runtime.GOOS {
	case "darwin":
		if home, err := os.UserHomeDir(); err == nil && home != "" {
			dirs = append(dirs, filepath.Join(home, "Library", "Application Support", "catalyst-code", "web"))
		}
	case "windows":
		if local := os.Getenv("LOCALAPPDATA"); local != "" {
			dirs = append(dirs, filepath.Join(local, "catalyst-code", "web"))
		}
		if home, err := os.UserHomeDir(); err == nil && home != "" {
			dirs = append(dirs, filepath.Join(home, "AppData", "Local", "catalyst-code", "web"))
		}
	default:
		dirs = append(dirs, "/opt/catalyst-code/web")
		if home, err := os.UserHomeDir(); err == nil && home != "" {
			dirs = append(dirs, filepath.Join(home, ".local", "share", "catalyst-code", "web"))
		}
	}
	return dirs
}

func webDirLooksInstalled(dir string) bool {
	if dir == "" {
		return false
	}
	if _, err := os.Stat(filepath.Join(dir, "start.js")); err != nil {
		return false
	}
	if _, err := os.Stat(filepath.Join(dir, "version.json")); err == nil {
		return true
	}
	_, err := os.Stat(filepath.Join(dir, "server.js"))
	return err == nil
}

// detectWebInstall returns the web bundle directory when the frontend appears
// installed (installer state or well-known paths).
func detectWebInstall() (webDir string, unitName string, ok bool) {
	if st, found := loadInstallerState(); found && st.WebInstalled {
		dir := st.WebDir
		if dir == "" {
			for _, d := range defaultWebDirs() {
				if webDirLooksInstalled(d) {
					dir = d
					break
				}
			}
		}
		if dir != "" && webDirLooksInstalled(dir) {
			return dir, st.UnitName, true
		}
		if dir != "" {
			return dir, st.UnitName, true
		}
	}
	for _, d := range defaultWebDirs() {
		if webDirLooksInstalled(d) {
			unit := defaultWebServiceName()
			if st, found := loadInstallerState(); found && st.UnitName != "" {
				unit = st.UnitName
			}
			return d, unit, true
		}
	}
	return "", "", false
}

func resolveInstallPrefix() string {
	if st, ok := loadInstallerState(); ok && st.Prefix != "" {
		return st.Prefix
	}
	if exe, err := os.Executable(); err == nil {
		if rp, e := filepath.EvalSymlinks(exe); e == nil {
			exe = rp
		}
		return filepath.Dir(exe)
	}
	return defaultInstallPrefix()
}

// isPrivileged reports whether the process can write root-owned system paths.
// On Windows this is always false (UAC elevation isn't modeled via Geteuid).
func isPrivileged() bool {
	if runtime.GOOS == "windows" {
		return false
	}
	return os.Geteuid() == 0
}

func elevationHint() string {
	if runtime.GOOS == "windows" {
		return "re-run from an elevated PowerShell: catcode --update"
	}
	return "re-run with sudo catcode --update"
}

// coreAssetName matches release-{linux,macos,windows}.sh core artifacts.
func coreAssetName(ver string) string {
	return fmt.Sprintf("catcode-core-%s-%s-%s%s", ver, osTag(), archTag(), coreExeSuffix())
}

func webAssetName(ver string) string {
	return fmt.Sprintf("catcode-web-%s.tar.gz", ver)
}

type webVersionJSON struct {
	Commit string `json:"commit"`
}

func readWebCommit(webDir string) string {
	for _, p := range []string{
		filepath.Join(webDir, "version.json"),
		filepath.Join(webDir, ".next", "version.json"),
	} {
		b, err := os.ReadFile(p)
		if err != nil {
			continue
		}
		var v webVersionJSON
		if json.Unmarshal(b, &v) == nil && v.Commit != "" {
			return v.Commit
		}
	}
	return ""
}

func commitsMatch(a, b string) bool {
	a = strings.TrimPrefix(strings.ToLower(strings.TrimSpace(a)), "v")
	b = strings.TrimPrefix(strings.ToLower(strings.TrimSpace(b)), "v")
	if a == "" || b == "" {
		return false
	}
	if a == b {
		return true
	}
	return strings.HasPrefix(a, b) || strings.HasPrefix(b, a)
}

// downloadVerifiedAsset fetches a release asset into a temp file and verifies
// its .sha256 sidecar when available. Caller must remove the returned path.
func downloadVerifiedAsset(rel *ghRelease, name string) (string, error) {
	asset := findAsset(rel, name)
	if asset == nil {
		return "", fmt.Errorf("no release asset named %s", name)
	}
	tmp, err := os.CreateTemp(os.TempDir(), "catcode-asset.*")
	if err != nil {
		return "", err
	}
	tmpName := tmp.Name()
	if err := download(asset.BrowserDownloadURL, name, tmp); err != nil {
		tmp.Close()
		os.Remove(tmpName)
		return "", err
	}
	if err := tmp.Close(); err != nil {
		os.Remove(tmpName)
		return "", err
	}
	if want, err := fetchSHA256(asset); err == nil {
		got, gerr := hashFile(tmpName)
		if gerr != nil {
			os.Remove(tmpName)
			return "", gerr
		}
		if !strings.EqualFold(want, got) {
			os.Remove(tmpName)
			return "", fmt.Errorf("checksum mismatch for %s", name)
		}
		fmt.Printf("  ✓ verified %s (sha256)\n", name)
	} else {
		fmt.Fprintf(os.Stderr, "  ! could not verify %s (%v) — proceeding without verification\n", name, err)
	}
	return tmpName, nil
}

func installBinaryAsset(rel *ghRelease, name, dest string) error {
	staged, err := downloadVerifiedAsset(rel, name)
	if err != nil {
		return err
	}
	defer os.Remove(staged)

	mode := os.FileMode(0o755)
	if fi, err := os.Stat(dest); err == nil {
		mode = fi.Mode()
	}
	if err := os.Chmod(staged, mode); err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(dest), 0o755); err != nil {
		return err
	}
	return selfReplace(staged, dest)
}

// extractTarGz clears destDir and extracts the gzip/tar archive into it.
func extractTarGz(archive, destDir string) error {
	if err := os.MkdirAll(destDir, 0o755); err != nil {
		return err
	}
	entries, err := os.ReadDir(destDir)
	if err != nil {
		return err
	}
	for _, e := range entries {
		if err := os.RemoveAll(filepath.Join(destDir, e.Name())); err != nil {
			return fmt.Errorf("clear %s: %w", e.Name(), err)
		}
	}

	f, err := os.Open(archive)
	if err != nil {
		return err
	}
	defer f.Close()
	gz, err := gzip.NewReader(f)
	if err != nil {
		return err
	}
	defer gz.Close()
	tr := tar.NewReader(gz)
	for {
		hdr, err := tr.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			return err
		}
		name := filepath.Clean(hdr.Name)
		if name == "." || name == "" {
			continue
		}
		if strings.HasPrefix(name, "..") || strings.Contains(name, string(filepath.Separator)+"..") {
			return fmt.Errorf("refusing unsafe path in archive: %s", hdr.Name)
		}
		target := filepath.Join(destDir, name)
		if !strings.HasPrefix(target, filepath.Clean(destDir)+string(os.PathSeparator)) && target != filepath.Clean(destDir) {
			return fmt.Errorf("refusing path outside dest: %s", hdr.Name)
		}
		switch hdr.Typeflag {
		case tar.TypeDir:
			if err := os.MkdirAll(target, 0o755); err != nil {
				return err
			}
		case tar.TypeReg:
			if err := os.MkdirAll(filepath.Dir(target), 0o755); err != nil {
				return err
			}
			out, err := os.OpenFile(target, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, os.FileMode(hdr.Mode)&0o777)
			if err != nil {
				return err
			}
			if _, err := io.Copy(out, tr); err != nil {
				out.Close()
				return err
			}
			out.Close()
		case tar.TypeSymlink:
			_ = os.Remove(target)
			if err := os.Symlink(hdr.Linkname, target); err != nil {
				return err
			}
		default:
		}
	}
	return nil
}

func writeWebVersionStamp(webDir, ver string) error {
	payload := map[string]any{
		"commit":     ver,
		"commitFull": ver,
		"dirty":      false,
		"builtAt":    "",
		"source":     "release",
	}
	b, err := json.MarshalIndent(payload, "", "  ")
	if err != nil {
		return err
	}
	b = append(b, '\n')
	paths := []string{filepath.Join(webDir, "version.json")}
	if _, err := os.Stat(filepath.Join(webDir, ".next")); err == nil {
		paths = append(paths, filepath.Join(webDir, ".next", "version.json"))
	}
	for _, pth := range paths {
		if err := os.WriteFile(pth, b, 0o644); err != nil {
			return err
		}
	}
	return nil
}

// stopWebService best-effort stops the web frontend so files can be replaced
// (critical on Windows where running node locks the tree).
func stopWebService(unitName string) {
	switch runtime.GOOS {
	case "windows":
		name := unitName
		if name == "" {
			name = windowsSvcName
		}
		if nssm, err := exec.LookPath("nssm"); err == nil {
			_ = exec.Command(nssm, "stop", name).Run()
		}
		_ = exec.Command("sc", "stop", name).Run()
		_ = exec.Command("schtasks", "/end", "/tn", windowsTaskName).Run()
	case "darwin":
		plist := launchdPlistPath()
		if _, err := os.Stat(plist); err == nil {
			_ = exec.Command("launchctl", "unload", plist).Run()
		}
	default:
		if unitName == "" {
			unitName = "catalyst-code-web.service"
		}
		if err := exec.Command("systemctl", "stop", unitName).Run(); err != nil {
			if sudo, lookErr := exec.LookPath("sudo"); lookErr == nil {
				_ = exec.Command(sudo, "-n", "systemctl", "stop", unitName).Run()
			}
		}
	}
}

func launchdPlistPath() string {
	home := os.Getenv("HOME")
	if home == "" {
		if h, err := os.UserHomeDir(); err == nil {
			home = h
		}
	}
	return filepath.Join(home, "Library", "LaunchAgents", "com.catalyst-code.web.plist")
}

func restartWebService(unitName string) error {
	switch runtime.GOOS {
	case "darwin":
		plist := launchdPlistPath()
		if _, err := os.Stat(plist); err != nil {
			fmt.Println("  ! launchd plist not found — restart the web service manually")
			return nil
		}
		_ = exec.Command("launchctl", "unload", plist).Run()
		cmd := exec.Command("launchctl", "load", plist)
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		return cmd.Run()

	case "windows":
		return restartWebServiceWindows(unitName)

	default:
		if unitName == "" {
			unitName = "catalyst-code-web.service"
		}
		cmd := exec.Command("systemctl", "restart", unitName)
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err == nil {
			return nil
		}
		sudo, lookErr := exec.LookPath("sudo")
		if lookErr != nil {
			return fmt.Errorf("systemctl restart failed and sudo not available")
		}
		cmd = exec.Command(sudo, "-n", "systemctl", "restart", unitName)
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("could not restart %s (try: sudo systemctl restart %s): %w", unitName, unitName, err)
		}
		return nil
	}
}

func restartWebServiceWindows(unitName string) error {
	name := unitName
	if name == "" {
		name = windowsSvcName
	}
	if nssm, err := exec.LookPath("nssm"); err == nil {
		cmd := exec.Command(nssm, "restart", name)
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err == nil {
			return nil
		}
		_ = exec.Command(nssm, "stop", name).Run()
		if err := exec.Command(nssm, "start", name).Run(); err == nil {
			return nil
		}
	}
	_ = exec.Command("sc", "stop", name).Run()
	if err := exec.Command("sc", "start", name).Run(); err == nil {
		return nil
	}
	_ = exec.Command("schtasks", "/end", "/tn", windowsTaskName).Run()
	cmd := exec.Command("schtasks", "/run", "/tn", windowsTaskName)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err == nil {
		return nil
	}
	return fmt.Errorf("could not restart Windows web service (tried nssm/%s, sc, schtasks/%s)", name, windowsTaskName)
}

// updateSiblingCore refreshes the catcode-core binary installed next to the
// TUI executable when this build relies on it (no embedded core) and the
// sibling exists. A missing sibling is fine ($CATCODE_CORE / dev layouts) and
// is skipped quietly. There is no version stamp on the core binary, so this
// always reinstalls from the release asset — cheap insurance against skew.
func updateSiblingCore(rel *ghRelease) error {
	if embeddedCoreAvailable {
		return nil
	}
	exe, err := os.Executable()
	if err != nil {
		return nil // can't locate ourselves; nothing safe to do
	}
	if rp, e := filepath.EvalSymlinks(exe); e == nil {
		exe = rp
	}
	coreDest := filepath.Join(filepath.Dir(exe), "catcode-core"+coreExeSuffix())
	if _, err := os.Stat(coreDest); err != nil {
		return nil // no sibling core installed; nothing to keep in sync
	}
	fmt.Printf("Updating catcode-core → %s\n", coreDest)
	if err := installBinaryAsset(rel, coreAssetName(rel.TagName), coreDest); err != nil {
		return err
	}
	fmt.Printf("  ✓ updated catcode-core → %s\n", rel.TagName)
	return nil
}

// updateCompanionComponents refreshes catcode-core + web bundle when the web
// frontend is installed. Returns whether web was touched.
func updateCompanionComponents(rel *ghRelease) (updatedWeb bool, err error) {
	webDir, unitName, ok := detectWebInstall()
	if !ok {
		fmt.Println("Web frontend: not installed (skipping)")
		// No web service, but on builds without an embedded core the TUI still
		// shells out to the sibling catcode-core. Refreshing only the CLI here
		// silently skews a new TUI against a stale core (missing protocol +
		// provider/plugin fixes), so keep the sibling in sync too.
		if err := updateSiblingCore(rel); err != nil {
			return false, fmt.Errorf("catcode-core: %w", err)
		}
		return false, nil
	}

	cur := readWebCommit(webDir)
	if cur != "" && commitsMatch(cur, rel.TagName) {
		fmt.Printf("Web frontend: already at %s (%s)\n", rel.TagName, webDir)
	} else if cur != "" {
		fmt.Printf("Updating web frontend %s → %s\n", cur, rel.TagName)
	} else {
		fmt.Printf("Installing web frontend → %s (%s)\n", rel.TagName, webDir)
	}

	if err := os.MkdirAll(webDir, 0o755); err != nil {
		if !isPrivileged() {
			return false, fmt.Errorf("cannot create %s — %s", webDir, elevationHint())
		}
		return false, fmt.Errorf("cannot create %s: %w", webDir, err)
	}
	if !canWriteDir(webDir) && !isPrivileged() {
		return false, fmt.Errorf("cannot write to %s — %s", webDir, elevationHint())
	}

	fmt.Println("Stopping web service…")
	stopWebService(unitName)

	prefix := resolveInstallPrefix()
	coreDest := filepath.Join(prefix, "catcode-core"+coreExeSuffix())
	fmt.Printf("Updating catcode-core → %s\n", coreDest)
	if err := installBinaryAsset(rel, coreAssetName(rel.TagName), coreDest); err != nil {
		_ = restartWebService(unitName)
		return false, fmt.Errorf("catcode-core: %w", err)
	}
	fmt.Printf("  ✓ updated catcode-core → %s\n", rel.TagName)

	webName := webAssetName(rel.TagName)
	fmt.Printf("Downloading %s…\n", webName)
	staged, err := downloadVerifiedAsset(rel, webName)
	if err != nil {
		_ = restartWebService(unitName)
		return false, fmt.Errorf("web bundle: %w", err)
	}
	defer os.Remove(staged)

	fmt.Printf("Extracting web bundle → %s\n", webDir)
	if err := extractTarGz(staged, webDir); err != nil {
		_ = restartWebService(unitName)
		return false, fmt.Errorf("extract web: %w", err)
	}
	if err := writeWebVersionStamp(webDir, rel.TagName); err != nil {
		fmt.Fprintf(os.Stderr, "  ! could not stamp version.json: %v\n", err)
	}
	if _, err := os.Stat(filepath.Join(webDir, "start.js")); err != nil {
		_ = restartWebService(unitName)
		return false, fmt.Errorf("web bundle missing start.js after extract")
	}
	fmt.Printf("  ✓ updated web → %s\n", rel.TagName)

	fmt.Println("Restarting web service…")
	if err := restartWebService(unitName); err != nil {
		fmt.Fprintf(os.Stderr, "  ! %v\n", err)
		fmt.Fprintf(os.Stderr, "    web files are updated; restart the service manually when ready.\n")
		return true, nil
	}
	fmt.Println("  ✓ web service restarted")
	return true, nil
}
