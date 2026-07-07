#!/usr/bin/env python3
"""vision-handoff pre_turn hook.

Fires before each model request. When the turn carries images and the active
model can't handle vision, remaps the turn to a vision-capable model.

Inputs (stdin JSON, in .args — requires pass_args: true in the manifest):
  model        str   the model about to be used
  has_images   bool  whether this turn attached images
  image_count  int   number of attached images
  models       list  discovered models, each {"id": ..., "vision": bool}
                    (the core merges the user-curated vision set into `vision`,
                    so this flag reflects endpoint + /vision config)
  vision_model str   preferred handoff target from the /vision config
                    ("" / absent = pick dynamically)

Environment (fallbacks, mainly for standalone use without the core config):
  VISION_MODEL   a single model id to hand off to
  VISION_MODELS  comma-separated ids that DO support vision

Output (stdout JSON):
  {"allow": true, "reason": "...", "modify": {"model": "<id>"}}   to swap
  {"allow": true, "reason": "..."}                                to keep

pre_turn is advisory: the core ignores `allow` and only honors modify.model,
so a failure here (missing python3, bad JSON, timeout) never blocks the turn —
it just means no handoff happens.
"""
import json
import os
import sys


def _emit(obj):
    sys.stdout.write(json.dumps(obj))
    sys.stdout.write("\n")


def _supports_vision(model_id, models, vision_models_set):
    if model_id in vision_models_set:
        return True
    for m in models:
        if m.get("id") == model_id:
            return bool(m.get("vision", False))
    return False


def _known_ids(models):
    return {m.get("id") for m in models if m.get("id")}


def _as_str(v):
    return v if isinstance(v, str) else ""


def main():
    try:
        ctx = json.load(sys.stdin)
    except Exception:
        # Unparseable context: don't block (advisory); no modification.
        _emit({"allow": True, "reason": "vision-handoff: unparseable context; no handoff"})
        return

    args = ctx.get("args") or {}
    model = _as_str(args.get("model", ""))
    has_images = bool(args.get("has_images", False))
    image_count = int(args.get("image_count", 0) or 0)
    models = args.get("models") or []
    ctx_model = _as_str(args.get("vision_model", "")).strip()

    # No images in this turn → nothing to hand off.
    if not has_images or image_count <= 0:
        _emit({"allow": True, "reason": "vision-handoff: no images; no handoff needed"})
        return

    vision_models = [x.strip() for x in os.environ.get("VISION_MODELS", "").split(",") if x.strip()]
    vision_models_set = set(vision_models)

    # Active model already handles vision → no handoff.
    if _supports_vision(model, models, vision_models_set):
        _emit({"allow": True, "reason": "vision-handoff: model '%s' supports vision; no handoff" % model})
        return

    known = _known_ids(models)
    chosen = None
    why = ""

    # 1) Preferred target from the /vision config (vision_model in context).
    if ctx_model and (not known or ctx_model in known):
        chosen = ctx_model
        why = "preferred target (vision config)"

    # 2) VISION_MODEL env (legacy / standalone fallback).
    if not chosen:
        user_model = os.environ.get("VISION_MODEL", "").strip()
        if user_model and (not known or user_model in known):
            chosen = user_model
            why = "user-specified (VISION_MODEL)"

    # 3) A model the endpoint/core flagged vision=true (includes /vision curation).
    if not chosen:
        for m in models:
            if m.get("vision"):
                chosen = m.get("id")
                why = "vision-capable model"
                break

    # 4) First declared VISION_MODELS env entry that's a known model.
    if not chosen:
        for mid in vision_models:
            if not known or mid in known:
                chosen = mid
                why = "declared via VISION_MODELS"
                break

    if not chosen:
        _emit({"allow": True, "reason": "vision-handoff: no vision-capable model found; staying on '%s'" % model})
        return

    _emit({
        "allow": True,
        "reason": "vision-handoff: %s: %s -> %s" % (why, model, chosen),
        "modify": {"model": chosen},
    })


if __name__ == "__main__":
    main()
