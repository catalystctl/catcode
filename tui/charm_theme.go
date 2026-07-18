package main

import (
	"charm.land/bubbles/v2/filepicker"
	"charm.land/bubbles/v2/help"
	"charm.land/bubbles/v2/list"
	"charm.land/huh/v2"
	"charm.land/lipgloss/v2"
)

// catalystHuhTheme maps the active Catalyst palette onto huh form chrome so
// confirms/inputs match the rest of the TUI (not stock Charm fuchsia).
func catalystHuhTheme() huh.Theme {
	return huh.ThemeFunc(func(isDark bool) *huh.Styles {
		t := huh.ThemeBase(isDark)
		accent := lipgloss.Color(c.accent)
		fg := lipgloss.Color(c.fg)
		dim := lipgloss.Color(c.secondary)
		errC := lipgloss.Color(c.err)
		bg := lipgloss.Color(c.bg)
		success := lipgloss.Color(c.success)

		t.Focused.Base = t.Focused.Base.BorderForeground(accent)
		t.Focused.Title = t.Focused.Title.Foreground(accent).Bold(true)
		t.Focused.Description = t.Focused.Description.Foreground(dim)
		t.Focused.ErrorIndicator = t.Focused.ErrorIndicator.Foreground(errC)
		t.Focused.ErrorMessage = t.Focused.ErrorMessage.Foreground(errC)
		t.Focused.SelectSelector = t.Focused.SelectSelector.Foreground(accent)
		t.Focused.Option = t.Focused.Option.Foreground(fg)
		t.Focused.FocusedButton = t.Focused.FocusedButton.Foreground(bg).Background(accent)
		t.Focused.BlurredButton = t.Focused.BlurredButton.Foreground(fg).Background(lipgloss.Color(c.dim))
		t.Focused.Next = t.Focused.FocusedButton
		t.Focused.TextInput.Cursor = t.Focused.TextInput.Cursor.Foreground(success)
		t.Focused.TextInput.Placeholder = t.Focused.TextInput.Placeholder.Foreground(dim)
		t.Focused.TextInput.Prompt = t.Focused.TextInput.Prompt.Foreground(accent)
		t.Focused.TextInput.Text = t.Focused.TextInput.Text.Foreground(fg)

		t.Blurred = t.Focused
		t.Blurred.Base = t.Focused.Base.BorderStyle(lipgloss.HiddenBorder())
		t.Group.Title = t.Focused.Title
		t.Group.Description = t.Focused.Description

		t.Help.ShortKey = keyHintStyle
		t.Help.ShortDesc = dimStyle
		t.Help.ShortSeparator = dimStyle
		t.Help.FullKey = keyHintStyle
		t.Help.FullDesc = dimStyle
		t.Help.FullSeparator = dimStyle
		t.Help.Ellipsis = dimStyle
		return t
	})
}

func catalystHelpStyles() help.Styles {
	st := help.DefaultStyles(themeIsDark())
	st.ShortKey = keyHintStyle
	st.ShortDesc = dimStyle
	st.ShortSeparator = dimStyle
	st.Ellipsis = dimStyle
	st.FullKey = keyHintStyle
	st.FullDesc = dimStyle
	st.FullSeparator = dimStyle
	return st
}

func catalystListDelegate() list.DefaultDelegate {
	d := list.NewDefaultDelegate()
	d.ShowDescription = true
	d.SetSpacing(0)
	d.Styles = list.NewDefaultItemStyles(themeIsDark())
	d.Styles.NormalTitle = baseStyle.Padding(0, 0, 0, 2)
	d.Styles.NormalDesc = dimStyle.Padding(0, 0, 0, 2)
	d.Styles.SelectedTitle = accentStyle.Padding(0, 0, 0, 1).
		Border(lipgloss.NormalBorder(), false, false, false, true).
		BorderForeground(lipgloss.Color(c.accent))
	d.Styles.SelectedDesc = dimStyle.Padding(0, 0, 0, 1).
		Border(lipgloss.NormalBorder(), false, false, false, true).
		BorderForeground(lipgloss.Color(c.accent))
	d.Styles.DimmedTitle = dimStyle.Padding(0, 0, 0, 2)
	d.Styles.DimmedDesc = dimStyle.Padding(0, 0, 0, 2)
	d.Styles.FilterMatch = accentStyle.Underline(true)
	return d
}

func catalystFilePickerStyles() filepicker.Styles {
	st := filepicker.DefaultStyles()
	st.Cursor = accentStyle
	st.Directory = accentStyle
	st.File = baseStyle
	st.Selected = accentStyle.Bold(true)
	st.Symlink = lipgloss.NewStyle().Foreground(lipgloss.Color(c.tool))
	st.DisabledFile = dimStyle
	st.Permission = dimStyle
	st.FileSize = dimStyle
	st.EmptyDirectory = dimStyle.SetString("  (empty)")
	return st
}
