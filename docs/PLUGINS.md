# Plugins

The full plugin authoring contract (hooks, tools, OAuth, memory providers, overrides)
lives as an opt-in skill so it is not injected into every system prompt:

**`.catalyst-code/skills/plugin-authoring/SKILL.md`**

(Also staged to `~/.catalyst-code/skills/plugin-authoring/SKILL.md` on first run.)

Apply it with `/skill:plugin-authoring`, or have the agent read that SKILL.md when
authoring or debugging a plugin.
