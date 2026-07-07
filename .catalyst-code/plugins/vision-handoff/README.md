# vision-handoff plugin

Hands an image-bearing turn off to a vision-capable model when the active model
can't handle vision.

## How it works

The harness core fires a `pre_turn` hook after building the user message
(including any attached images) and before the first model request. This
plugin's `pre_turn` hook receives the active `model`, whether the turn has
images (`has_images` / `image_count`), and the discovered `models` list (each
with a `vision` flag). If the turn has images and the active model lacks
vision, it returns `modify: { "model": "<vision-model-id>" }` and the core swaps
the model for that turn (validating the id against discovered models).

`pre_turn` is **advisory**: a broken hook never blocks the turn — it just means
no handoff happens.

## Configuration (environment variables)

- `VISION_MODEL=<id>` — pin a single target model to hand off to (highest
  priority). Use this when you always want a specific vision model.
- `VISION_MODELS=a,b,c` — comma-separated model ids that support vision. This
  declares vision capability for models whose `/models/info` endpoint doesn't
  advertise a `capabilities.vision` flag, and also seeds the dynamic pick.

### Precedence when choosing a vision model

1. `VISION_MODEL` (if set and a known model)
2. the first model flagged `vision: true` by the endpoint
3. the first `VISION_MODELS` entry that's a known model

If none of those yield a model, the turn stays on the active model (no handoff).

## Triggering it

Attach an image and the handoff triggers automatically:

- `/attach <path-to-image.png> [prompt]` — explicit attach (built-in).
- Type or paste a path to an image file in the input — the TUI auto-detects
  existing image files (`.png .jpg .jpeg .gif .webp .bmp .svg .tif .tiff`) by
  extension and attaches them.
- `@mention` an image file — the `@path/to/image.png` token is detected the same
  way (the `@` prefix is stripped).

## Requirements

- `python3` on `PATH` (the hook is `hooks/pre_turn.py`). If absent, the hook
  fails to spawn and the turn proceeds with the original model (graceful).

## Loading

The core scans `.catalyst-code/plugins` at startup, so **restart the TUI**
after adding this plugin. Verify with `/plugin-config` (it should list
`vision-handoff`), then press enter on a plugin to toggle it on or off.
