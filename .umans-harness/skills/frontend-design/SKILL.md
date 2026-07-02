---
name: frontend-design
description: Distinctive, intentional visual design when building new UI or reshaping an existing one — palette, typography, layout, motion, and copy. Calibrates against the templated "AI-design defaults" and forces a concrete token plan before code. Use for any surface that should carry a real visual identity; skip for design-system-constrained admin/CRUD work.
version: 1
---

# Frontend Design

Act as the design lead at a small studio known for giving every client a visual
identity that could not be mistaken for anyone else's. This client has already
rejected proposals that felt templated and is paying for a distinctive point of
view: make deliberate, opinionated choices about palette, typography, and layout
that are specific to this brief, and take one real aesthetic risk you can justify.

The bar is not "looks designed." The bar is "could only have been made for this
subject." If your plan would survive being swapped onto a different brief with a
find-and-replace, it is not done.

## When to use

- Building a new surface that should carry identity (landing pages, marketing,
  product launch moments, a brand-new app).
- Reshaping an existing UI that has drifted into generic territory.
- Any brief that names a visual direction or asks for "distinctive."

## When NOT to use

This skill is the wrong tool, and applying it harms the work, when:

- You are working **inside an existing design system** — extend its tokens, do
  not invent a rival palette. If a token set already exists, your job is
  coherence, not self-expression.
- The surface is **internal / admin / CRUD** where speed and legibility beat
  identity. A settings table does not need a signature element.
- The target is **email** or another client with rigid, broken CSS support.
- A **brand book** or style guide already dictates palette/type — follow it
  exactly; the brief's words always win.

For everything else, proceed.

## The subject is the source

If the brief does not pin down what the product is, pin it down yourself before
designing: name one concrete subject, its audience, and the page's single job,
and state the choice. Use anything in memory about the human's preferences or
past designs as a hint.

Distinctive choices come from the **subject's own world** — its materials,
instruments, artifacts, and vernacular — not from a personal taste for a
particular aesthetic. Move from subject to design language:

> **Letterpress studio** → kerning cues, ink traps, registration crosses, the
> bite of the paper; pull *those* into the type treatment, the dividers, the
> cursor. **Maritime navigation** → charts, bearings, the rotation interval of
> a real lighthouse; encode real data into the signature. **Audio synthesis** →
> waveforms, patch cables, knob detents; let the controls look like the gear.

The test: the tokens should be **unembarrassable** to someone who knows the
subject. If a domain expert would find them random or merely decorative, they
are not grounded yet. Build with the brief's real content throughout —
placeholder lorem-ipsum is how designs start reading as templates.

## The hero is a thesis

Open with the most characteristic thing in the subject's world — a headline, an
image, an animation, a live demo, an interactive moment — in whatever form makes
sense for it. Be deliberate:

> **Default you'd reach for:** centered headline + gradient-text subhead + two
> CTAs over a blurred aurora blob.
> **Deliberate for a letterpress-studio brief:** full-bleed close-up of a forme
> locking up, headline set in the studio's own wood-type face, kerned to the
> registration marks — the subject *is* the hero.

A big number with a small label, supporting stats, and a gradient accent is the
template answer; use it only if it is truly the best option for this brief.

## Defaults to justify-or-replace

AI-generated design right now clusters around a small set of looks. They are all
**legitimate for some briefs** — including, sometimes, the brief in front of you
— but they are **defaults, not choices**: they appear regardless of subject.
Where the brief pins down a direction, follow it exactly (even if it asks for one
of these). Where it leaves an axis free, do not spend that freedom on a default.

- **Editorial Cream** — `#F4F1EA` ground, high-contrast serif display, terracotta accent.
- **Acid Dark** — near-black ground, one neon (acid-green or vermilion) accent.
- **Broadsheet** — hairline rules, zero border-radius, dense newspaper columns.
- **Bento Glass** — `rounded-2xl` translucent cards over blurred gradient blobs.
- **Gradient-Text Hero** — `bg-clip-text` color gradient on the H1.
- **Lucide Card Grid** — three or four outline-icon feature cards in a row.
- **Aurora Centered Trio** — centered headline + subhead + two CTAs over blurred blobs.

**Gate:** if your plan contains two or more of these, revise before writing code.
Each one you keep must be a justified choice for *this* brief, not the path of
least resistance. (As for any hired designer, there is a balance between leaning
on what you do well and treating each project as a chance to experiment.)

## The design plan (commit it before code)

Do not start in markup. First commit a compact **token system** as a real
artifact — write it down in whatever form the project uses (CSS custom
properties, a design-tokens JSON, a theme/Tailwind config). Derive every color,
font, and spacing decision in the build from this plan.

- **Color** — 4–6 named hex values, each with a job (ground, ink, accent, …).
- **Type** — two or more roles: a characterful **display** face used with
  restraint, a complementary **body** face, and a **utility** face for
  captions/data if needed. Set a real type scale (sizes + weights + tracking),
  not "big heading, small text." Pair deliberately — not the same families you
  would reach for on any other project.
- **Layout** — one concept, stated as a sentence, plus an ASCII wireframe to
  compare against an alternative or two.
- **Signature** — the single unique element this page will be remembered by,
  that embodies the brief. It should be **unembarrassable** (see above), not a
  gimmick. If you cannot say what the signature *does* for the subject, cut it.

```
/* token artifact — adapt to your stack; placeholders, not real values */
--ground: #…;  --ink: #…;  --accent: #…;          /* color: each with a job */
--display: "…", serif;  --body: "…", sans;         /* type roles */
--step-0: …;  --step-1: …;  --step-2: …;           /* type scale */
/* signature: <one sentence naming the memorable element and why it fits> */
```

## Process: brainstorm → plan → reverse-5 → build → screenshot → critique

1. **Brainstorm** the token system above from the brief.
2. **Review the plan against the brief** before building: does each part read as
   a choice made for *this* brief, or the generic default you would produce for
   any similar page? Revise what is generic; note what you changed and why.
3. **Reverse-5 (the uniqueness check).** List five ways a templated generator
   would approach *this same brief*. If your plan matches more than two of them,
   revise those axes. (This replaces the vague instinct to "see if you'd arrive
   somewhere similar" with a step you can actually run.)
4. **Build** following the revised plan exactly, deriving every decision from the
   tokens. Watch CSS selector specificity — it is easy to emit a type-based rule
   (`.section`) and an element-based rule (`.cta`) that cancel each other, often
   on padding/margin between sections.
5. **Render → screenshot → compare.** Treat screenshots as a first-class step,
   not a "if your environment supports it." Answer from the image: does the
   signature read? Does any element read as a default? A picture is worth a
   thousand tokens — revise against what you see, then re-shoot.
6. **Critique again** before finishing (see Restraint).

Do most of the planning and iteration in your thinking; surface ideas to the
user only when you have reasonable confidence they will delight them.

## Restraint and self-critique

Spend your boldness in **one** place. Let the signature be the one memorable
thing, keep everything around it quiet and disciplined, and cut any decoration
that does not serve the brief. Not taking a risk can be a risk itself — but a
page with five bold moves reads as noise, not point of view. Build to the
quality floor below without announcing it. As Chanel advised: before leaving the
house, look in the mirror and remove one accessory. Keep quick notes on what you
have tried so you do not repeat yourself across passes.

## Motion

Leverage motion deliberately. An orchestrated moment usually lands harder than
scattered effects — but extra animation is one of the clearest tells of
AI-generated design, so default toward less.

**Decision rule:** motion is justified when it does one of —

- (a) explains a state change the user just caused,
- (b) guides attention on first load to the one thing that matters,
- (c) **is** the signature itself.

Otherwise, static. When you do animate, honor `prefers-reduced-motion` (see
floor). Ambient atmosphere (a slow drift, a parallax) is the riskiest category —
use only if it *is* the signature.

## Accessibility floor (non-negotiable)

Reach this floor without announcing it; never trade it for aesthetic:

- **Contrast** — text at WCAG-AA against its background (4.5:1 body, 3:1 large).
- **Focus** — visible `:focus-visible` styles; never `outline: none` without a
  replacement.
- **Motion** — honor `prefers-reduced-motion`; gate non-essential animation.
- **Color scheme** — if dark mode is in scope, support `prefers-color-scheme`
  (or an explicit toggle); do not ship a single fixed scheme.
- **Responsive** — works at 320px; no horizontal scroll; tap targets ≥ 44px.
- **Semantics** — landmark elements, headings in order, `alt` text, `aria-label`
  on icon-only controls.

## Structure is information

Structural devices — numbering, eyebrows, dividers, side notes, drop caps —
should **encode something true** about the content, not decorate it. The original
skill flags `01 / 02 / 03` markers; the principle is broader: tracked-caps
eyebrows like "FEATURES," breadcrumb dividers, and margin notes all count. Ask
whether the device carries information the reader needs (a real sequence, a typed
timeline, a hierarchy). If it is purely ornamental, remove it.

## Writing in design

Words appear in a design for one reason: to make it easier to understand, and
therefore easier to use. They are design material, not decoration. Bring the same
intentionality to copy as to spacing and color.

- **Write from the end user's side of the screen.** Name things by what people
  control and recognize, never by how the system is built. A person manages
  notifications, not webhook config. Be specific, not clever.
- **Active voice as default.** A control says exactly what happens when used:
  "Save changes," not "Submit." An action keeps its name through the flow — the
  button "Publish" produces a toast "Published."
- **Treat failure and emptiness as direction, not mood.** Explain what went
  wrong and how to fix it, in the interface's voice. Errors do not apologize and
  are never vague. An empty screen is an invitation to act.
- **Match the copy's register to the visual register.** A maximalist design
  wants maximalist copy cadence; a quiet, disciplined design wants telegraphic
  copy. When copy and visual disagree, the whole thing reads as assembled, not
  designed.
- Plain verbs, sentence case, no filler, tone matched to brand and audience.
  Let each element do exactly one job: a label labels, an example demonstrates,
  nothing quietly does double duty.

## Currency

These defaults drift fast. The cluster above is the AI-design default-look set
as of mid-2025; before designing, take a moment to re-derive the *current*
default-cluster (what generators are over-producing right now) and add it to the
justify-or-replace list. A skill that names last year's defaults and misses this
year's is worse than no list at all.
