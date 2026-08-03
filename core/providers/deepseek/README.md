# DeepSeek API — Official Provider Bundle

This is the first-party DeepSeek API provider bundle for the harness. It is
kept under `core/providers/deepseek/` so the provider can be removed without
touching the rest of the provider catalog, matching the Kimi bundle layout.

## Setup

Use the built-in API-key login flow:

```text
/login deepseek
```

Paste a DeepSeek API key when prompted, or configure a provider entry with
`api_key_env: "DEEPSEEK_API_KEY"`.

| Setting | Value |
|---|---|
| Provider id | `deepseek` |
| API key env var | `DEEPSEEK_API_KEY` |
| Wire protocol | OpenAI Chat Completions |
| API base | `https://api.deepseek.com` |
| Model list | `GET /models` |
| Chat endpoint | `POST /chat/completions` |

The harness treats the live `/models` response as authoritative, so newly
published DeepSeek model ids appear automatically. DeepSeek's model endpoint
only publishes ids and ownership; known model capabilities are curated from
the official model documentation and unknown/new ids are enriched from the
runtime `models.dev` registry (with a safe fallback when either endpoint is
unavailable). Capabilities include context length, maximum output, reasoning,
thinking levels, and vision support.

DeepSeek thinking requests use the vendor's OpenAI-compatible
`thinking: {"type": "enabled|disabled"}` field plus `reasoning_effort`, and
streamed `reasoning_content` is shown as harness thinking output.

## References

- [DeepSeek API documentation](https://api-docs.deepseek.com/)
- [List models](https://api-docs.deepseek.com/api/list-models)
- [Models and pricing](https://api-docs.deepseek.com/quick_start/pricing)
- [Chat completions](https://api-docs.deepseek.com/api/create-chat-completion)
