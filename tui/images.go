package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// imageExtensions are the file extensions treated as attachable images when
// found in a user's message (typed, pasted, or @-mentioned).
var imageExtensions = map[string]bool{
	".png": true, ".jpg": true, ".jpeg": true, ".gif": true,
	".webp": true, ".bmp": true, ".svg": true, ".tif": true, ".tiff": true,
}

// maxAttachImageBytes guards against slurping a huge file into a base64 data URL.
const maxAttachImageBytes = 20 * 1024 * 1024 // 20 MiB

// extractImagePaths scans text for whitespace-delimited tokens that refer to
// existing image files and returns their absolute paths. A leading "@" (an
// @-mention) and surrounding quotes are stripped, so both "@shots/ui.png" and
// a pasted "/abs/path/screen.jpg" are recognized. Non-image tokens and
// missing/oversized/non-file paths are ignored. Duplicates are de-duplicated.
func extractImagePaths(text string) []string {
	var imgs []string
	seen := make(map[string]bool)
	for _, tok := range strings.Fields(text) {
		p := strings.TrimPrefix(tok, "@")
		p = strings.Trim(p, "\"")
		if p == "" {
			continue
		}
		ext := strings.ToLower(filepath.Ext(p))
		if !imageExtensions[ext] {
			continue
		}
		abs, err := filepath.Abs(p)
		if err != nil {
			continue
		}
		info, err := os.Stat(abs)
		if err != nil || info.IsDir() {
			continue
		}
		if info.Size() > maxAttachImageBytes {
			continue
		}
		if seen[abs] {
			continue
		}
		seen[abs] = true
		imgs = append(imgs, abs)
	}
	return imgs
}

// withImages detects image file paths mentioned in text and, if any exist on
// disk, adds them to payload's "images" array so the core builds a multimodal
// message (and the vision-handoff plugin can route accordingly). It merges
// with any images already present (e.g. from /attach) without duplicates.
// Non-image text is left untouched. Returns the (possibly mutated) payload.
func (s *session) withImages(payload map[string]any, text string) map[string]any {
	imgs := extractImagePaths(text)
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
	s.logInfo(fmt.Sprintf("attaching %d image(s): %s", len(merged), strings.Join(merged, ", ")))
	return payload
}
