package main

import "strings"

// ---- Symbol tables for LaTeX → Unicode pretty-printing ---------------------

var greekLetters = map[string]string{
	"alpha": "α", "beta": "β", "gamma": "γ", "delta": "δ", "epsilon": "ε",
	"varepsilon": "ε", "zeta": "ζ", "eta": "η", "theta": "θ", "vartheta": "ϑ",
	"iota": "ι", "kappa": "κ", "lambda": "λ", "mu": "μ", "nu": "ν",
	"xi": "ξ", "pi": "π", "varpi": "ϖ", "rho": "ρ", "varrho": "ϱ",
	"sigma": "σ", "varsigma": "ς", "tau": "τ", "upsilon": "υ", "phi": "φ",
	"varphi": "ϕ", "chi": "χ", "psi": "ψ", "omega": "ω",
	"Gamma": "Γ", "Delta": "Δ", "Theta": "Θ", "Lambda": "Λ",
	"Xi": "Ξ", "Pi": "Π", "Sigma": "Σ", "Upsilon": "Υ",
	"Phi": "Φ", "Psi": "Ψ", "Omega": "Ω",
}

var mathOps = map[string]string{
	"sum": "∑", "int": "∫", "oint": "∮", "prod": "∏", "coprod": "∐",
	"infty": "∞", "partial": "∂", "nabla": "∇", "pm": "±", "mp": "∓",
	"times": "×", "div": "÷", "cdot": "·", "ast": "∗", "star": "⋆",
	"leq": "≤", "le": "≤", "geq": "≥", "ge": "≥", "neq": "≠", "ne": "≠",
	"approx": "≈", "equiv": "≡", "sim": "∼", "simeq": "≃", "cong": "≅",
	"propto": "∝", "ll": "≪", "gg": "≫", "prec": "≺", "succ": "≻",
	"to": "→", "rightarrow": "→", "Rightarrow": "⇒", "implies": "⇒",
	"leftarrow": "←", "Leftarrow": "⇐", "leftrightarrow": "↔",
	"mapsto": "↦", "uparrow": "↑", "downarrow": "↓",
	"in": "∈", "notin": "∉", "ni": "∋", "subset": "⊂", "supset": "⊃",
	"subseteq": "⊆", "supseteq": "⊇", "cup": "∪", "cap": "∩",
	"setminus": "∖", "emptyset": "∅", "varnothing": "∅",
	"forall": "∀", "exists": "∃", "nexists": "∄", "neg": "¬", "lnot": "¬",
	"land": "∧", "lor": "∨", "oplus": "⊕", "ominus": "⊖", "otimes": "⊗",
	"odot": "⊙", "wedge": "∧", "vee": "∨",
	"perp": "⊥", "angle": "∠", "measuredangle": "∡",
	"dots": "…", "ldots": "…", "cdots": "⋯", "vdots": "⋮", "ddots": "⋱",
	"deg": "°", "circ": "∘", "bullet": "•",
	"hbar": "ℏ", "ell": "ℓ", "Re": "ℜ", "Im": "ℑ", "aleph": "ℵ",
	"langle": "⟨", "rangle": "⟩",
}

// blackboard maps \mathbb{X} bodies to their double-struck glyphs.
var blackboard = map[string]string{
	"R": "ℝ", "Z": "ℤ", "N": "ℕ", "Q": "ℚ", "C": "ℂ", "H": "ℍ",
	"P": "ℙ", "A": "𝔸", "B": "𝔹", "D": "𝔻", "E": "𝔼", "F": "𝔽",
	"G": "𝔾", "J": "𝕁", "K": "𝕂", "L": "𝕃", "S": "𝕊", "T": "𝕋",
	"U": "𝕌", "V": "𝕍", "W": "𝕎", "X": "𝕏", "Y": "𝕐",
}

// ---- Super/subscript maps --------------------------------------------------

var superscriptMap = map[rune]rune{
	'0': '⁰', '1': '¹', '2': '²', '3': '³', '4': '⁴',
	'5': '⁵', '6': '⁶', '7': '⁷', '8': '⁸', '9': '⁹',
	'+': '⁺', '-': '⁻', '=': '⁼', '(': '⁽', ')': '⁾',
	'a': 'ᵃ', 'b': 'ᵇ', 'c': 'ᶜ', 'd': 'ᵈ', 'e': 'ᵉ', 'f': 'ᶠ',
	'g': 'ᵍ', 'h': 'ʰ', 'i': 'ⁱ', 'j': 'ʲ', 'k': 'ᵏ', 'l': 'ˡ',
	'm': 'ᵐ', 'n': 'ⁿ', 'o': 'ᵒ', 'p': 'ᵖ', 'r': 'ʳ', 's': 'ˢ',
	't': 'ᵗ', 'u': 'ᵘ', 'v': 'ᵛ', 'w': 'ʷ', 'x': 'ˣ', 'y': 'ʸ', 'z': 'ᶻ',
	'A': 'ᴬ', 'B': 'ᴮ', 'D': 'ᴰ', 'E': 'ᴱ', 'G': 'ᴳ', 'H': 'ᴴ',
	'I': 'ᴵ', 'J': 'ᴶ', 'K': 'ᴷ', 'L': 'ᴸ', 'M': 'ᴹ', 'N': 'ᴺ',
	'O': 'ᴼ', 'P': 'ᴾ', 'R': 'ʳ', 'T': 'ᵀ', 'U': 'ᵁ', 'V': 'ⱽ', 'W': 'ᵂ',
}

var subscriptMap = map[rune]rune{
	'0': '₀', '1': '₁', '2': '₂', '3': '₃', '4': '₄',
	'5': '₅', '6': '₆', '7': '₇', '8': '₈', '9': '₉',
	'+': '₊', '-': '₋', '=': '₌', '(': '₍', ')': '₎',
	'a': 'ₐ', 'e': 'ₑ', 'h': 'ₕ', 'i': 'ᵢ', 'j': 'ⱼ', 'k': 'ₖ',
	'l': 'ₗ', 'm': 'ₘ', 'n': 'ₙ', 'o': 'ₒ', 'p': 'ₚ', 'r': 'ᵣ',
	's': 'ₛ', 't': 'ₜ', 'u': 'ᵤ', 'v': 'ᵥ', 'x': 'ₓ',
}

func toSuperscript(s string) string {
	var b strings.Builder
	for _, c := range s {
		if r, ok := superscriptMap[c]; ok {
			b.WriteRune(r)
		} else {
			b.WriteRune(c) // fall back to the original char (still readable)
		}
	}
	return b.String()
}

func toSubscript(s string) string {
	var b strings.Builder
	for _, c := range s {
		if r, ok := subscriptMap[c]; ok {
			b.WriteRune(r)
		} else {
			b.WriteRune(c)
		}
	}
	return b.String()
}
