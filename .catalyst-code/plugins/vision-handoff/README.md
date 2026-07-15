# vision-handoff plugin

Hands an image-bearing turn off to a **cheapest same-provider** vision-capable
model when the active model can't handle vision. Recommended **ON** by default
(`enabled` in `.catalyst-code/vision.json`).

## How it works

The harness core fires a `pre_turn` hook after building the user message
(including any attached images) and before the first model request. This
plugin's `pre_turn` hook receives:

- `model`, `has_images` / `image_count`
- `models` (each with `vision`, `provider`, `cost_rank`)
- `enabled` (first-class toggle; default true)
- `vision_model` (optional preferred pin)
- `recommended_vision_model` (core-ranked cheapest same-provider candidate)

If the turn has images, handoff is enabled, and the active model lacks vision,
it returns `modify: { "model": "<vision-model-id>" }` and the core swaps the
model for that turn. If the plugin is missing/`python3` absent, the **core
still applies** `recommended_vision_model` as a fallback.

`pre_turn` is **advisory**: a broken hook never blocks the turn.

## Configuration

Persisted in `.catalyst-code/vision.json` (Settings / `/vision`):

```json
{
  "enabled": true,
  "vision_model": null,
  "vision_models": []
}
```

- `enabled` — auto handoff (default **true**, recommended ON).
- `vision_model` — pin a preferred target (overrides cheapest pick).
- `vision_models` — curated ids treated as vision-capable (merged with endpoint flags).

### Environment overrides

- `VISION_MODEL=<id>` — pin a target (highest priority after preferred config).
- `VISION_MODELS=a,b,c` — declare vision capability / seed fallbacks.

### Selection precedence

1. Preferred `vision_model` from config (if known)
2. `VISION_MODEL` env
3. Core `recommended_vision_model` (cheapest same-provider)
4. Local cheapest same-provider among `vision: true`
5. First `VISION_MODELS` entry

Never crosses providers for the automatic cheapest pick (preferred pin may).

## Triggering it

Attach an image and the handoff triggers automatically:

- `/attach <path-to-image.png> [prompt]` — explicit attach (built-in).
- Type or paste a path to an image file in the input — the TUI auto-detects
  existing image files (`.png .jpg .jpeg .gif .webp .bmp .svg .tif .tiff`) by
  extension and attaches them.
- `@mention` an image file — the `@path/to/image.png` token is detected the same
  way (the `@` prefix is stripped).

## Requirements

- `python3` on `PATH` for the hook script (optional — core fallback still works).

## Loading

The core scans `.catalyst-code/plugins` at startup / stages the plugin into
`~/.catalyst-code/` on first run. Toggle via Settings or `/plugin-config`.
