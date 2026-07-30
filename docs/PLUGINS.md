# Plugins

The full plugin authoring contract (hooks, tools, OAuth, memory providers, overrides)
lives as an opt-in skill so it is not injected into every system prompt:

**`.catalyst-code/skills/plugin-authoring/SKILL.md`**

(Also staged to `~/.catalyst-code/skills/plugin-authoring/SKILL.md` on first run.)

Apply it with `/skill:plugin-authoring`, or have the agent read that SKILL.md when
authoring or debugging a plugin.

## Prompt-backed commands

A plugin command can be **prompt-backed** (`prompt_file` + `mode: "agent_turn"`)
instead of script-backed: the harness renders the template and submits it as a
normal agent turn (same path as a user `send`), so a single slash command like
`/deep-research` can drive a full multi-agent workflow with no script, no
interpreter, and no recompile. See the "Prompt-backed commands" section of the
plugin-authoring skill. The bundled `deep-research` plugin is the reference
implementation.
