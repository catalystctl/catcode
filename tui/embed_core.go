//go:build embed_core

package main

import (
	_ "embed"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
)

// embeddedCore is the umans-core binary compiled into this executable by the
// macOS standalone build (release-macos.sh, -tags embed_core). It lets the
// single downloaded file run from any CWD with no separate umans-core.
//
//go:embed embed/umans-core
var embeddedCore []byte

// embeddedCorePath extracts the embedded core to a per-user cache dir keyed by
// the bundled version + target os/arch, marks it executable, and returns its
// path. Idempotent: an existing extraction whose size matches is reused. On any
// failure it returns "" so coreBinaryPath falls back to its normal search.
func embeddedCorePath() string {
	dir, err := os.UserCacheDir()
	if err != nil || dir == "" {
		home, hErr := os.UserHomeDir()
		if hErr != nil || home == "" {
			return ""
		}
		dir = filepath.Join(home, ".cache")
	}
	cacheDir := filepath.Join(dir, "umans-harness")
	if err := os.MkdirAll(cacheDir, 0o755); err != nil {
		return ""
	}
	name := fmt.Sprintf("umans-core-%s-%s-%s", coreVersion, runtime.GOOS, runtime.GOARCH)
	dst := filepath.Join(cacheDir, name)
	if fi, err := os.Stat(dst); err == nil && !fi.IsDir() && fi.Size() == int64(len(embeddedCore)) {
		return dst
	}
	tmp, err := os.CreateTemp(cacheDir, name+".*.tmp")
	if err != nil {
		return ""
	}
	tmpName := tmp.Name()
	cleanup := func() { _ = os.Remove(tmpName) }
	if _, err := tmp.Write(embeddedCore); err != nil {
		_ = tmp.Close()
		cleanup()
		return ""
	}
	if err := tmp.Chmod(0o755); err != nil {
		_ = tmp.Close()
		cleanup()
		return ""
	}
	if err := tmp.Close(); err != nil {
		cleanup()
		return ""
	}
	if err := os.Rename(tmpName, dst); err != nil {
		cleanup()
		return ""
	}
	return dst
}
