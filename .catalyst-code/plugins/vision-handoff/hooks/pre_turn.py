#!/usr/bin/env python3
"""vision-handoff pre_turn hook.

Fires before each model request. When the turn carries images and the active
model can't handle vision, remaps the turn to a vision-capable model on the
same provider (cheapest by default; preferred pin wins).

Inputs (stdin JSON, in .args — requires pass_args: true in the manifest):
  model                     str   the model about to be used
  has_images                bool  whether this turn attached images
  image_count               int   number of attached images
  models                    list  discovered models:
                                  {"id", "vision", "provider", "cost_rank"}
  vision_model              str   preferred handoff target from /vision config
                                  ("" / absent = use recommended / cheapest)
  enabled                   bool  first-class handoff toggle (default true)
  provider                  str   active model's provider
  recommended_vision_model  str   core-ranked cheapest same-provider candidate

Environment (fallbacks, mainly for standalone use without the core config):
  VISION_MODEL   a single model id to hand off to (overrides recommended)
  VISION_MODELS  comma-separated ids that DO support vision

Output (stdout JSON):
  {"allow": true, "reason": "...", "modify": {"model": "<id>"}}   to swap
  {"allow": true, "reason": "..."}                                to keep

pre_turn is advisory: the core ignores `allow` and only honors modify.model,
so a failure here (missing python3, bad JSON, timeout) never blocks the turn —
the core still applies `recommended_vision_model` as a fallback.
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


def _cost_rank(m):
    if isinstance(m.get("cost_rank"), (int, float)):
        return int(m["cost_rank"])
    mid = _as_str(m.get("id")).lower()
    # Expensive first when substrings collide (e.g. "ultra" contains "lite").
    if any(x in mid for x in ("opus", "ultra")) or mid.endswith("max") or "-max" in mid:
        return 80
    if "o1" in mid or "o3" in mid:
        return 80
    if any(x in mid for x in ("nano", "haiku", "mini", "flash-lite", "flash_lite")):
        return 10
    if "flash" in mid or "-lite" in mid or "_lite" in mid or "small" in mid or "fast" in mid:
        return 20
    if any(x in mid for x in ("sonnet", "codex", "gpt-4o")):
        return 40
    if "-pro" in mid or "pro-" in mid or mid.endswith("pro") or "medium" in mid:
        return 50
    return 60


def _same_provider(active_provider, m):
    return _as_str(m.get("provider")) == _as_str(active_provider)


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
    enabled = args.get("enabled", True)
    if enabled is None:
        enabled = True
    active_provider = _as_str(args.get("provider", ""))
    recommended = _as_str(args.get("recommended_vision_model", "")).strip()

    # No images in this turn → nothing to hand off.
    if not has_images or image_count <= 0:
        _emit({"allow": True, "reason": "vision-handoff: no images; no handoff needed"})
        return

    if not enabled:
        _emit({"allow": True, "reason": "vision-handoff: disabled in vision config; no handoff"})
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

    # 3) Core-ranked cheapest same-provider recommendation.
    if not chosen and recommended and (not known or recommended in known):
        chosen = recommended
        why = "cheapest same-provider (core)"

    # 4) Local cheapest same-provider pick among vision-capable models.
    if not chosen:
        cands = [
            m for m in models
            if m.get("vision") and m.get("id") and m.get("id") != model
            and _same_provider(active_provider, m)
        ]
        cands.sort(key=lambda m: (_cost_rank(m), _as_str(m.get("id"))))
        if cands:
            chosen = cands[0].get("id")
            why = "cheapest same-provider"

    # 5) First declared VISION_MODELS env entry that's a known model.
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
