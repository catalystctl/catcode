package main

import (
	"crypto/sha256"
	"fmt"
	"strings"
	"sync"

	"charm.land/glamour/v2"
	"charm.land/glamour/v2/ansi"
	styles "charm.land/glamour/v2/styles"
)

// ---------------------------------------------------------------------------
// Markdown via Glamour
//
// Chat replies use Glamour (goldmark + chroma) for CommonMark/GFM. Styles are
// remapped onto the active Catalyst theme. Plain multiline (no markdown
// markers) stays on the lightweight legacy renderer. Rendered blocks are
// cached by content hash + width + theme.
// ---------------------------------------------------------------------------

const glamourBlockCacheMax = 256

var (
	glamourMu         sync.Mutex
	glamourWidth      int
	glamourRenderer   *glamour.TermRenderer
	glamourThemeKey   string
	glamourBlockCache = map[string]string{}
)

func renderMarkdown(text string, w int) string {
	if w < 8 {
		w = 8
	}
	if looksLikeMarkdown(text) {
		out, err := renderMarkdownGlamour(text, w)
		if err == nil {
			return out
		}
	}
	return renderMarkdownLegacy(text, w)
}

func looksLikeMarkdown(text string) bool {
	return strings.Contains(text, "```") ||
		strings.Contains(text, "**") ||
		strings.Contains(text, "__") ||
		strings.Contains(text, "`") ||
		strings.Contains(text, "](") ||
		strings.HasPrefix(strings.TrimSpace(text), "#") ||
		strings.Contains(text, "\n- ") ||
		strings.Contains(text, "\n* ") ||
		strings.Contains(text, "\n> ") ||
		strings.Contains(text, "\n| ")
}

func renderMarkdownGlamour(text string, w int) (string, error) {
	key := glamourBlockKey(text, w)
	glamourMu.Lock()
	if out, ok := glamourBlockCache[key]; ok {
		glamourMu.Unlock()
		return out, nil
	}
	glamourMu.Unlock()

	r, err := glamourRendererFor(w)
	if err != nil {
		return "", err
	}
	out, err := r.Render(chatHardBreaks(text))
	if err != nil {
		return "", err
	}
	out = strings.Trim(out, "\n")
	glamourMu.Lock()
	if len(glamourBlockCache) >= glamourBlockCacheMax {
		glamourBlockCache = map[string]string{}
	}
	glamourBlockCache[key] = out
	glamourMu.Unlock()
	return out, nil
}

func glamourBlockKey(text string, w int) string {
	sum := sha256.Sum256([]byte(fmt.Sprintf("%s|%d|%s", activeTheme.name, w, text)))
	return fmt.Sprintf("%x", sum[:16])
}

func chatHardBreaks(text string) string {
	lines := strings.Split(text, "\n")
	var b strings.Builder
	inFence := false
	for i, line := range lines {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "```") {
			inFence = !inFence
			b.WriteString(line)
			if i < len(lines)-1 {
				b.WriteByte('\n')
			}
			continue
		}
		b.WriteString(line)
		if i < len(lines)-1 {
			if inFence {
				b.WriteByte('\n')
			} else {
				b.WriteString("  \n")
			}
		}
	}
	return b.String()
}

func glamourRendererFor(w int) (*glamour.TermRenderer, error) {
	glamourMu.Lock()
	defer glamourMu.Unlock()
	key := activeTheme.name + "|" + c.fg + "|" + c.accent + "|" + c.tool
	if glamourRenderer != nil && glamourWidth == w && glamourThemeKey == key {
		return glamourRenderer, nil
	}
	style := catalystGlamourStyle()
	r, err := glamour.NewTermRenderer(
		glamour.WithStyles(style),
		glamour.WithWordWrap(w),
	)
	if err != nil {
		return nil, err
	}
	glamourRenderer = r
	glamourWidth = w
	glamourThemeKey = key
	return r, nil
}

func catalystGlamourStyle() ansi.StyleConfig {
	base := styles.DarkStyleConfig
	if !themeIsDark() {
		base = styles.LightStyleConfig
	}
	zero := uint(0)
	base.Document.Margin = &zero
	base.Document.BlockPrefix = ""
	base.Document.BlockSuffix = ""
	base.Document.Color = strPtr(c.fg)

	base.Heading.Color = strPtr(c.accent)
	base.H1.Color = strPtr(c.accent)
	base.H1.BackgroundColor = nil
	base.H2.Color = strPtr(c.accent)
	base.H3.Color = strPtr(c.user)
	base.Link.Color = strPtr(c.accent)
	base.LinkText.Color = strPtr(c.accent)
	base.Code.Color = strPtr(c.tool)
	base.Code.BackgroundColor = strPtr(c.dim)
	base.CodeBlock.Color = strPtr(c.muted)
	base.CodeBlock.Margin = &zero
	base.Emph.Color = strPtr(c.fg)
	base.Strong.Color = strPtr(c.fg)
	base.BlockQuote.Color = strPtr(c.secondary)
	base.HorizontalRule.Color = strPtr(c.decor)
	base.Item.Color = strPtr(c.fg)
	base.Enumeration.Color = strPtr(c.accent)

	// Chroma tokens → Catalyst palette (syntax highlight in fences).
	if base.CodeBlock.Chroma == nil {
		base.CodeBlock.Chroma = &ansi.Chroma{}
	}
	ch := base.CodeBlock.Chroma
	ch.Text.Color = strPtr(c.fg)
	ch.Error.Color = strPtr(c.err)
	ch.Comment.Color = strPtr(c.secondary)
	ch.CommentPreproc.Color = strPtr(c.warn)
	ch.Keyword.Color = strPtr(c.accent)
	ch.KeywordReserved.Color = strPtr(c.user)
	ch.KeywordNamespace.Color = strPtr(c.assist)
	ch.KeywordType.Color = strPtr(c.tool)
	ch.Operator.Color = strPtr(c.warn)
	ch.Punctuation.Color = strPtr(c.muted)
	ch.Name.Color = strPtr(c.fg)
	ch.NameBuiltin.Color = strPtr(c.tool)
	ch.NameTag.Color = strPtr(c.accent)
	ch.NameAttribute.Color = strPtr(c.user)
	ch.NameClass.Color = strPtr(c.accent)
	ch.NameDecorator.Color = strPtr(c.warn)
	ch.NameFunction.Color = strPtr(c.success)
	ch.LiteralNumber.Color = strPtr(c.success)
	ch.LiteralString.Color = strPtr(c.tool)
	ch.LiteralStringEscape.Color = strPtr(c.warn)
	ch.GenericDeleted.Color = strPtr(c.err)
	ch.GenericEmph.Color = strPtr(c.fg)
	ch.GenericInserted.Color = strPtr(c.success)
	ch.GenericStrong.Color = strPtr(c.fg)
	ch.GenericSubheading.Color = strPtr(c.accent)
	ch.Background.Color = strPtr(c.bg)
	return base
}

func strPtr(s string) *string { return &s }

func invalidateGlamourCache() {
	glamourMu.Lock()
	defer glamourMu.Unlock()
	glamourRenderer = nil
	glamourThemeKey = ""
	glamourBlockCache = map[string]string{}
}
