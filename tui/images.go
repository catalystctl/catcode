package main

import (
	"bytes"
	"encoding/base64"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	tea "charm.land/bubbletea/v2"
)

// imageExtensions are the file extensions treated as attachable images when
// found in a user's message (typed, pasted, or @-mentioned).
var imageExtensions = map[string]bool{
	".png": true, ".jpg": true, ".jpeg": true, ".gif": true,
	".webp": true, ".bmp": true, ".svg": true, ".tif": true, ".tiff": true,
}

// maxAttachImageBytes guards against slurping a huge file into a base64 data URL.
const maxAttachImageBytes = 20 * 1024 * 1024 // 20 MiB

// maxPendingImages caps how many images can be staged in the composer at once.
const maxPendingImages = 8

// ---------------------------------------------------------------------------
// Path extraction (typed / pasted / @-mentioned file paths)
// ---------------------------------------------------------------------------

// extractImagePaths scans text for whitespace-delimited tokens that refer to
// existing image files and returns their absolute paths. A leading "@" (an
// @-mention) and surrounding quotes are stripped, so both "@shots/ui.png" and
// a pasted "/abs/path/screen.jpg" are recognized. Non-image tokens and
// missing/oversized/non-file paths are ignored. Duplicates are de-duplicated.
func extractImagePaths(text string) []string {
	var imgs []string
	seen := make(map[string]bool)
	for _, tok := range tokenizePaths(text) {
		if abs, ok := resolveImagePath(tok); ok {
			if seen[abs] {
				continue
			}
			seen[abs] = true
			imgs = append(imgs, abs)
		}
	}
	return imgs
}

// tokenizePaths splits text into candidate path tokens. Handles quoted strings
// (so paths with spaces work) and strips a leading @ (mention syntax).
func tokenizePaths(text string) []string {
	var out []string
	// First pull out double- and single-quoted spans so spaces inside them survive.
	rest := text
	for len(rest) > 0 {
		dq := strings.IndexByte(rest, '"')
		sq := strings.IndexByte(rest, '\'')
		qi := -1
		qc := byte(0)
		if dq >= 0 && (sq < 0 || dq < sq) {
			qi, qc = dq, '"'
		} else if sq >= 0 {
			qi, qc = sq, '\''
		}
		if qi < 0 {
			for _, tok := range strings.Fields(rest) {
				out = append(out, strings.TrimPrefix(tok, "@"))
			}
			break
		}
		// Unquoted prefix.
		for _, tok := range strings.Fields(rest[:qi]) {
			out = append(out, strings.TrimPrefix(tok, "@"))
		}
		// Quoted span.
		end := strings.IndexByte(rest[qi+1:], qc)
		if end < 0 {
			// Unclosed quote: treat the rest as one token.
			out = append(out, strings.TrimPrefix(strings.TrimSpace(rest[qi+1:]), "@"))
			break
		}
		out = append(out, strings.TrimPrefix(rest[qi+1:qi+1+end], "@"))
		rest = rest[qi+1+end+1:]
	}
	return out
}

// resolveImagePath normalizes a candidate token (file://, ~/, relative, bare
// path) into an absolute existing image path, or returns ok=false.
func resolveImagePath(tok string) (string, bool) {
	p := strings.TrimSpace(tok)
	p = strings.TrimPrefix(p, "@")
	p = strings.Trim(p, "\"'")
	if p == "" {
		return "", false
	}
	// file:// URIs (some terminals/VS Code paste these for drag-drop).
	if strings.HasPrefix(p, "file://") {
		p = strings.TrimPrefix(p, "file://")
		// file:///C:/Users/... on Windows-style; file:///home/... on Unix.
		if runtime.GOOS != "windows" && len(p) >= 3 && p[0] == '/' && p[2] == ':' {
			// Leave Windows drive paths alone only when running on Windows.
		}
		// Percent-decode minimal cases (%20 for spaces).
		p = strings.ReplaceAll(p, "%20", " ")
	}
	if strings.HasPrefix(p, "~/") {
		if home, err := os.UserHomeDir(); err == nil {
			p = filepath.Join(home, p[2:])
		}
	}
	ext := strings.ToLower(filepath.Ext(p))
	// Strip trailing punctuation often left by drag-drop / chat paste (path., path,)
	if !imageExtensions[ext] && len(ext) > 1 {
		trim := strings.TrimRight(p, ".,;:)]}>")
		ext = strings.ToLower(filepath.Ext(trim))
		if imageExtensions[ext] {
			p = trim
		}
	}
	if !imageExtensions[ext] {
		return "", false
	}
	abs, err := filepath.Abs(p)
	if err != nil {
		return "", false
	}
	info, err := os.Stat(abs)
	if err != nil || info.IsDir() {
		return "", false
	}
	if info.Size() > maxAttachImageBytes {
		return "", false
	}
	return abs, true
}

// validateImage checks that p is a readable image file of an allowed type and
// within the size limit, returning its absolute path. Used by /attach (the main
// send paths validate via withImages -> extractImagePaths); /attach sets
// "images" directly so it must validate the explicit path itself (P2-12).
func validateImage(p string) (string, error) {
	ext := strings.ToLower(filepath.Ext(p))
	if !imageExtensions[ext] {
		return "", fmt.Errorf("unsupported image type %q (png/jpg/jpeg/gif/webp/bmp/svg/tif/tiff)", ext)
	}
	abs, err := filepath.Abs(p)
	if err != nil {
		return "", err
	}
	info, err := os.Stat(abs)
	if err != nil {
		return "", fmt.Errorf("image not found: %s", p)
	}
	if info.IsDir() {
		return "", fmt.Errorf("not a file: %s", p)
	}
	if info.Size() > maxAttachImageBytes {
		return "", fmt.Errorf("image too large (%d bytes; max %d)", info.Size(), maxAttachImageBytes)
	}
	return abs, nil
}

// withImages detects image file paths mentioned in text and, if any exist on
// disk, adds them to payload's "images" array so the core builds a multimodal
// message (and the vision-handoff plugin can route accordingly). It merges
// with any images already present (e.g. from /attach) and any pending pasted
// attachments without duplicates. Non-image text is left untouched.
func (s *session) withImages(payload map[string]any, text string) map[string]any {
	imgs := extractImagePaths(text)
	// Include staged paste/clipboard attachments (paths or data URLs).
	if len(s.pendingImages) > 0 {
		imgs = append(imgs, s.pendingImages...)
	}
	if len(imgs) == 0 {
		return payload
	}
	existing, _ := payload["images"].([]string)
	seen := make(map[string]bool, len(existing)+len(imgs))
	merged := make([]string, 0, len(existing)+len(imgs))
	for _, p := range existing {
		if !seen[p] {
			seen[p] = true
			merged = append(merged, p)
		}
	}
	for _, p := range imgs {
		if !seen[p] {
			seen[p] = true
			merged = append(merged, p)
		}
	}
	payload["images"] = merged
	s.logInfo(fmt.Sprintf("attaching %d image(s): %s", len(merged), summarizeImageRefs(merged)))
	return payload
}

// summarizeImageRefs shortens image refs for log lines (data URLs are huge).
func summarizeImageRefs(imgs []string) string {
	parts := make([]string, 0, len(imgs))
	for _, p := range imgs {
		if strings.HasPrefix(p, "data:") {
			// data:image/png;base64,...
			media := "image"
			if i := strings.Index(p, ";"); i > 5 {
				media = p[5:i]
			}
			parts = append(parts, fmt.Sprintf("<%s data-url>", media))
		} else {
			parts = append(parts, p)
		}
	}
	return strings.Join(parts, ", ")
}

// ---------------------------------------------------------------------------
// Pending attachments (pasted / clipboard images staged for the next send)
// ---------------------------------------------------------------------------

// addPendingImage stages a path or data URL for the next send. Returns false
// when the attachment was rejected (cap, duplicate, empty).
func (s *session) addPendingImage(ref string) bool {
	ref = strings.TrimSpace(ref)
	if ref == "" {
		return false
	}
	for _, p := range s.pendingImages {
		if p == ref {
			return false
		}
	}
	if len(s.pendingImages) >= maxPendingImages {
		s.logError(fmt.Sprintf("too many attached images (max %d)", maxPendingImages))
		return false
	}
	s.pendingImages = append(s.pendingImages, ref)
	return true
}

// clearPendingImages drops all staged paste/clipboard attachments.
func (s *session) clearPendingImages() {
	s.pendingImages = nil
}

// popPendingImage removes the last staged attachment (for backspace-when-empty).
func (s *session) popPendingImage() bool {
	if len(s.pendingImages) == 0 {
		return false
	}
	s.pendingImages = s.pendingImages[:len(s.pendingImages)-1]
	return true
}

// pendingImageLabel is a short chip label for the i-th pending attachment.
func (s *session) pendingImageLabel(i int) string {
	if i < 0 || i >= len(s.pendingImages) {
		return ""
	}
	ref := s.pendingImages[i]
	if strings.HasPrefix(ref, "data:") {
		return fmt.Sprintf("[Image %d]", i+1)
	}
	base := filepath.Base(ref)
	if base == "" || base == "." || base == "/" {
		return fmt.Sprintf("[Image %d]", i+1)
	}
	return fmt.Sprintf("[Image %d: %s]", i+1, base)
}

// ---------------------------------------------------------------------------
// Paste inspection — turn clipboard / bracketed-paste payloads into attachments
// ---------------------------------------------------------------------------

// pasteImageResult is what handlePasteContent returns after inspecting a paste.
type pasteImageResult struct {
	// paths/data URLs that were staged as pending attachments
	attached []string
	// remaining text that should still go into the text input (may be empty)
	text string
	// true when the paste was entirely consumed as image(s) (no text residual)
	consumed bool
}

// handlePasteContent inspects pasted content for images. Over SSH / VS Code /
// any terminal, images arrive as:
//   - bracketed-paste of a remote file path (extension wrote /tmp/…png)
//   - file:// URI
//   - data:image/...;base64,... URL
//   - raw base64 (single line) that decodes to PNG/JPEG/GIF/WEBP/BMP
//   - raw image bytes (rare; some terminals dump binary into the paste buffer)
//
// When images are found they are staged on s.pendingImages and stripped from
// the residual text so the composer doesn't fill with base64 garbage.
func (s *session) handlePasteContent(content string) pasteImageResult {
	content = normalizePasteContent(content)
	if content == "" {
		return pasteImageResult{}
	}

	// 1) Raw binary image bytes (paste buffer starts with image magic).
	if ext := sniffImageExt([]byte(content)); ext != "" {
		if path, err := saveImageBytes([]byte(content), ext); err == nil {
			if s.addPendingImage(path) {
				s.logSuccess(fmt.Sprintf("attached image → %s", filepath.Base(path)))
				return pasteImageResult{attached: []string{path}, consumed: true}
			}
		}
	}

	// 2) data:image/...;base64,... (single or multi URL, possibly with text).
	if attached, rest := extractDataURLImages(content); len(attached) > 0 {
		var kept []string
		for _, ref := range attached {
			if s.addPendingImage(ref) {
				kept = append(kept, ref)
			}
		}
		if len(kept) > 0 {
			s.logSuccess(fmt.Sprintf("attached %d image(s) from paste", len(kept)))
			rest = strings.TrimSpace(rest)
			return pasteImageResult{attached: kept, text: rest, consumed: rest == ""}
		}
	}

	// 3) Whole paste is pure base64 of an image (common for clipboard bridges).
	if path, ok := tryDecodeBase64Image(content); ok {
		if s.addPendingImage(path) {
			s.logSuccess(fmt.Sprintf("attached image → %s", filepath.Base(path)))
			return pasteImageResult{attached: []string{path}, consumed: true}
		}
	}

	// 4) Whole paste is one image path / file:// URI (drag-drop, VS Code ext).
	if abs, ok := resolveImagePath(strings.TrimSpace(content)); ok {
		if s.addPendingImage(abs) {
			s.logSuccess(fmt.Sprintf("attached image → %s", filepath.Base(abs)))
			return pasteImageResult{attached: []string{abs}, consumed: true}
		}
	}

	// 5) Multi-token paste where every non-empty token is an image path —
	// common when several screenshots are dropped at once.
	tokens := tokenizePaths(content)
	if len(tokens) > 0 {
		allImages := true
		var paths []string
		for _, tok := range tokens {
			if abs, ok := resolveImagePath(tok); ok {
				paths = append(paths, abs)
			} else if strings.TrimSpace(tok) != "" {
				allImages = false
				break
			}
		}
		if allImages && len(paths) > 0 {
			var kept []string
			for _, p := range paths {
				if s.addPendingImage(p) {
					kept = append(kept, p)
				}
			}
			if len(kept) > 0 {
				s.logSuccess(fmt.Sprintf("attached %d image(s)", len(kept)))
				return pasteImageResult{attached: kept, consumed: true}
			}
		}
	}

	// Not an image-only paste — leave text for the input. extractImagePaths
	// still picks up any path tokens at send time.
	return pasteImageResult{text: content}
}

// normalizePasteContent strips bracketed-paste artifacts and BOM.
func normalizePasteContent(s string) string {
	s = strings.TrimPrefix(s, "\ufeff")
	// Some terminals leave trailing CR.
	s = strings.ReplaceAll(s, "\r\n", "\n")
	s = strings.ReplaceAll(s, "\r", "\n")
	return s
}

// sniffImageExt returns a file extension (including dot) when b starts with a
// recognized image magic sequence; otherwise "".
func sniffImageExt(b []byte) string {
	if len(b) < 4 {
		return ""
	}
	switch {
	case bytes.HasPrefix(b, []byte{0x89, 'P', 'N', 'G'}):
		return ".png"
	case bytes.HasPrefix(b, []byte{0xFF, 0xD8, 0xFF}):
		return ".jpg"
	case bytes.HasPrefix(b, []byte("GIF8")):
		return ".gif"
	case len(b) >= 12 && bytes.HasPrefix(b, []byte("RIFF")) && string(b[8:12]) == "WEBP":
		return ".webp"
	case bytes.HasPrefix(b, []byte{0x42, 0x4D}):
		return ".bmp"
	case bytes.HasPrefix(b, []byte("<svg")) || bytes.HasPrefix(b, []byte("<?xml")):
		// Loose SVG sniff — only accept if it looks like SVG content.
		if bytes.Contains(b[:min(512, len(b))], []byte("<svg")) {
			return ".svg"
		}
	}
	return ""
}

// saveImageBytes writes image bytes to a temp file under the system temp dir
// and returns the absolute path.
func saveImageBytes(data []byte, ext string) (string, error) {
	if len(data) == 0 {
		return "", fmt.Errorf("empty image")
	}
	if len(data) > maxAttachImageBytes {
		return "", fmt.Errorf("image too large (%d bytes; max %d)", len(data), maxAttachImageBytes)
	}
	if ext == "" {
		ext = ".png"
	}
	dir := filepath.Join(os.TempDir(), "catcode-paste")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return "", err
	}
	name := fmt.Sprintf("paste-%d%s", time.Now().UnixNano(), ext)
	path := filepath.Join(dir, name)
	if err := os.WriteFile(path, data, 0o600); err != nil {
		return "", err
	}
	return path, nil
}

// extractDataURLImages pulls data:image/...;base64,... URLs out of text and
// returns them plus the residual text with those URLs removed.
func extractDataURLImages(text string) (urls []string, rest string) {
	const prefix = "data:image/"
	restBuf := text
	for {
		i := strings.Index(restBuf, prefix)
		if i < 0 {
			break
		}
		// Find the end of the data URL (whitespace or end).
		j := i + len(prefix)
		for j < len(restBuf) {
			c := restBuf[j]
			if c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '"' || c == '\'' {
				break
			}
			j++
		}
		candidate := restBuf[i:j]
		if strings.Contains(candidate, ";base64,") && len(candidate) > len(prefix)+20 {
			// Validate that the payload is real base64 of an image (or at least
			// decodes). Keep the data URL form so the core can passthrough.
			if comma := strings.Index(candidate, ","); comma > 0 {
				raw, err := base64.StdEncoding.DecodeString(candidate[comma+1:])
				if err == nil && (sniffImageExt(raw) != "" || len(raw) > 32) {
					if len(raw) <= maxAttachImageBytes {
						urls = append(urls, candidate)
						// Prefer saving to a file so logs/UI stay small.
						if ext := sniffImageExt(raw); ext != "" {
							if path, err := saveImageBytes(raw, ext); err == nil {
								urls[len(urls)-1] = path
							}
						}
					}
				}
			}
		}
		restBuf = restBuf[:i] + restBuf[j:]
	}
	return urls, restBuf
}

// tryDecodeBase64Image treats the whole string as base64 image data. Returns
// a temp file path when successful.
func tryDecodeBase64Image(s string) (string, bool) {
	s = strings.TrimSpace(s)
	// Strip optional data-URL wrapper already handled; here we want pure b64.
	if strings.HasPrefix(s, "data:") {
		return "", false
	}
	// Only attempt on "looks like base64" content to avoid decoding code pastes.
	if len(s) < 64 || len(s) > maxAttachImageBytes*2 {
		return "", false
	}
	// Single-line or whitespace-only wrapping; reject if it has many mixed tokens.
	compact := strings.Map(func(r rune) rune {
		if r == '\n' || r == '\r' || r == ' ' || r == '\t' {
			return -1
		}
		return r
	}, s)
	if len(compact) < 64 {
		return "", false
	}
	for _, c := range compact {
		if !((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') ||
			(c >= '0' && c <= '9') || c == '+' || c == '/' || c == '=' || c == '-') {
			return "", false
		}
	}
	// Prefer StdEncoding; fall back to RawStd / URL encodings.
	var raw []byte
	var err error
	if raw, err = base64.StdEncoding.DecodeString(compact); err != nil {
		if raw, err = base64.RawStdEncoding.DecodeString(compact); err != nil {
			if raw, err = base64.URLEncoding.DecodeString(compact); err != nil {
				return "", false
			}
		}
	}
	ext := sniffImageExt(raw)
	if ext == "" {
		return "", false
	}
	path, err := saveImageBytes(raw, ext)
	if err != nil {
		return "", false
	}
	return path, true
}

// ---------------------------------------------------------------------------
// Local clipboard image grab (works when a display/clipboard is available;
// over plain SSH this usually fails — paste path / data URL is the SSH path)
// ---------------------------------------------------------------------------

// clipboardImageMsg is delivered after an async clipboard image read.
type clipboardImageMsg struct {
	path string
	err  error
}

// readClipboardImageCmd returns a tea.Cmd that tries to read an image from the
// local system clipboard (macOS/Linux/Windows). Safe to call over SSH — it
// fails fast with a clear error when no clipboard image is available.
func readClipboardImageCmd() tea.Cmd {
	return func() tea.Msg {
		path, err := readClipboardImage()
		return clipboardImageMsg{path: path, err: err}
	}
}

// readClipboardImage shells out to platform clipboard tools to fetch PNG (or
// equivalent) image bytes. Returns a temp file path.
func readClipboardImage() (string, error) {
	var data []byte
	var err error
	switch runtime.GOOS {
	case "darwin":
		data, err = readClipboardImageDarwin()
	case "linux", "freebsd", "openbsd", "netbsd":
		data, err = readClipboardImageLinux()
	case "windows":
		data, err = readClipboardImageWindows()
	default:
		return "", fmt.Errorf("clipboard images unsupported on %s", runtime.GOOS)
	}
	if err != nil {
		return "", err
	}
	ext := sniffImageExt(data)
	if ext == "" {
		return "", fmt.Errorf("clipboard does not contain a recognized image")
	}
	return saveImageBytes(data, ext)
}

func readClipboardImageDarwin() ([]byte, error) {
	// pngpaste is the cleanest tool when installed.
	if path, err := exec.LookPath("pngpaste"); err == nil {
		out, err := exec.Command(path, "-").CombinedOutput()
		if err == nil && sniffImageExt(out) != "" {
			return out, nil
		}
	}
	// osascript → write clipboard picture to a temp file.
	tmp := filepath.Join(os.TempDir(), fmt.Sprintf("catcode-clip-%d.png", time.Now().UnixNano()))
	script := fmt.Sprintf(`set theFile to POSIX file %q
try
  set pngData to the clipboard as «class PNGf»
  set fileRef to open for access theFile with write permission
  write pngData to fileRef
  close access fileRef
on error errMsg
  try
    close access theFile
  end try
  error errMsg
end try`, tmp)
	cmd := exec.Command("osascript", "-e", script)
	if out, err := cmd.CombinedOutput(); err != nil {
		_ = os.Remove(tmp)
		return nil, fmt.Errorf("macOS clipboard: %v (%s)", err, strings.TrimSpace(string(out)))
	}
	data, err := os.ReadFile(tmp)
	_ = os.Remove(tmp)
	if err != nil {
		return nil, err
	}
	if sniffImageExt(data) == "" {
		return nil, fmt.Errorf("macOS clipboard has no image (copy a screenshot first)")
	}
	return data, nil
}

func readClipboardImageLinux() ([]byte, error) {
	// Wayland first.
	if path, err := exec.LookPath("wl-paste"); err == nil {
		out, err := exec.Command(path, "--type", "image/png").CombinedOutput()
		if err == nil && sniffImageExt(out) != "" {
			return out, nil
		}
		// Some compositors expose other mime types.
		for _, mime := range []string{"image/jpeg", "image/webp", "image/bmp", "image/gif"} {
			out, err := exec.Command(path, "--type", mime).CombinedOutput()
			if err == nil && sniffImageExt(out) != "" {
				return out, nil
			}
		}
	}
	// X11 via xclip.
	if path, err := exec.LookPath("xclip"); err == nil {
		out, err := exec.Command(path, "-selection", "clipboard", "-t", "image/png", "-o").CombinedOutput()
		if err == nil && sniffImageExt(out) != "" {
			return out, nil
		}
		for _, mime := range []string{"image/jpeg", "image/webp", "image/bmp", "image/gif"} {
			out, err := exec.Command(path, "-selection", "clipboard", "-t", mime, "-o").CombinedOutput()
			if err == nil && sniffImageExt(out) != "" {
				return out, nil
			}
		}
	}
	// xsel doesn't support image targets well; skip.
	return nil, fmt.Errorf("no image in clipboard (need wl-paste or xclip with an image; over SSH paste a path or use a terminal/VS Code extension that injects the image)")
}

func readClipboardImageWindows() ([]byte, error) {
	// PowerShell: pull Clipboard image, save as PNG to a temp path, print path.
	tmp := filepath.Join(os.TempDir(), fmt.Sprintf("catcode-clip-%d.png", time.Now().UnixNano()))
	// Escape for PowerShell single-quoted string (double any single quotes).
	psTmp := strings.ReplaceAll(tmp, "'", "''")
	script := fmt.Sprintf(`
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$img = [System.Windows.Forms.Clipboard]::GetImage()
if ($null -eq $img) { Write-Error 'no image in clipboard'; exit 1 }
$img.Save('%s', [System.Drawing.Imaging.ImageFormat]::Png)
Write-Output '%s'
`, psTmp, psTmp)
	cmd := exec.Command("powershell", "-NoProfile", "-NonInteractive", "-Command", script)
	out, err := cmd.CombinedOutput()
	if err != nil {
		_ = os.Remove(tmp)
		return nil, fmt.Errorf("Windows clipboard: %v (%s)", err, strings.TrimSpace(string(out)))
	}
	data, err := os.ReadFile(tmp)
	_ = os.Remove(tmp)
	if err != nil {
		return nil, err
	}
	if sniffImageExt(data) == "" {
		return nil, fmt.Errorf("Windows clipboard has no image")
	}
	return data, nil
}
