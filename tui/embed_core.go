//go:build embed_core

package main

import (
	"crypto/sha256"
	_ "embed"
	"encoding/hex"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"runtime"
)

// embeddedCore is the catcode-core binary compiled into this executable by the
// macOS standalone build (release-macos.sh, -tags embed_core). It lets the
// single downloaded file run from any CWD with no separate catcode-core.
//
//go:embed embed/catcode-core
var embeddedCore []byte

// embeddedCoreAvailable reports whether a core is compiled into this binary.
const embeddedCoreAvailable = true

// embeddedCoreHash caches the SHA-256 of the embedded core (computed once).
var embeddedCoreHash string

// coreContentHash returns the SHA-256 hex of the embedded core binary.
func coreContentHash() string {
	if embeddedCoreHash == "" {
		h := sha256.Sum256(embeddedCore)
		embeddedCoreHash = hex.EncodeToString(h[:])
	}
	return embeddedCoreHash
}

// sha256File returns the SHA-256 hex of the file at p.
func sha256File(p string) (string, error) {
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
	cacheDir := filepath.Join(dir, "catalyst-code")
	if err := os.MkdirAll(cacheDir, 0o755); err != nil {
		return ""
	}
	name := fmt.Sprintf("catcode-core-%s-%s-%s%s", coreVersion, runtime.GOOS, runtime.GOARCH, coreExeSuffix())
	dst := filepath.Join(cacheDir, name)
	// On Windows the extracted core needs the .exe suffix (coreExeSuffix) so
	// CreateProcess exec's it by name and AV tools recognize it; on macOS/Linux
	// coreExeSuffix() is "" so the cache name is unchanged from prior releases.
	// Reuse an existing extraction only if it matches the embedded core by BOTH
	// size AND content hash, so a tampered/replaced cache file (same size, different
	// bytes) can't be exec'd (P2: was size-only — a TOCTOU on the shared cache).
	if fi, err := os.Stat(dst); err == nil && !fi.IsDir() && fi.Size() == int64(len(embeddedCore)) {
		if h, err := sha256File(dst); err == nil && h == coreContentHash() {
			return dst
		}
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
