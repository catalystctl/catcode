//! Protocol-agnostic message types.  This module replaces the prior practice of
//! storing the entire conversation as `Vec<serde_json::Value>` (implicitly
//! OpenAI chat-completions shaped).  Every message is now a first-class Rust
//! type so that the harness can speak both OpenAI `/chat/completions` and
//! Anthropic `/v1/messages` natively — each provider path converts *from* these
//! types directly into its own wire format, instead of translating JSON→JSON.
//!
//! **Persistence compatibility:** serialisation uses serde with `#[serde(tag =
//! "role")]` so the on-disk JSONL format is **byte-for-byte identical** to the
//! old `Vec<Value>` format.  Old sessions load seamlessly; new sessions can be
//! read by an older harness without any migration.
//!
//! **Tolerant deserialization:** the [`Message::try_from`] impl uses a custom
//! helper that coerces / defaults a few fields providers sometimes emit in
//! shapes that do not match the strong type (e.g. `arguments` as a JSON object
//! instead of a string, or assistant `content` as a multimodal array).  This
//! keeps a single malformed tool-call or content block from aborting the
//! whole conversation deserialization, and mirrors the sanitizers in
//! `provider::sanitize_tool_call_arguments` which used to run *after* the
//! number-crunching `Value` parse.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// One message in the agent conversation — the canonical, provider-agnostic
/// format used everywhere except the actual HTTP wire bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role")]
#[serde(rename_all = "lowercase")]
pub enum Message {
    #[serde(rename = "system")]
    System {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        content: Content,
    },

    #[serde(rename = "user")]
    User {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        content: Content,
    },

    #[serde(rename = "assistant")]
    Assistant {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Assistant text content.  Providers may emit this as a JSON array of
        /// text blocks (multimodal-shaped assistant content), as a plain string,
        /// or omit it entirely when only tool_calls are present.
        /// `coerce_optional_text` accepts a string or a multimodal array and
        /// joins the text parts into one string so the field is always a string.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "coerce_optional_text"
        )]
        content: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "reasoning_content"
        )]
        thinking: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<ToolCall>>,
    },

    #[serde(rename = "tool")]
    Tool {
        tool_call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        content: String,
    },
}

/// Message content — either a plain string or a multimodal array of parts
/// (text blocks / inline images).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Multimodal(Vec<ContentPart>),
}

/// One part of a multimodal user message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    Image { image_url: ImageUrl },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// An assistant tool-call entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default)]
    pub typ: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    #[serde(default)]
    pub name: String,
    /// Tool-call arguments, always stored as a JSON **string**.  Some providers
    /// emit this as a JSON object instead of a string, and some omit it entirely.
    /// The custom deserializer normalizes all of these to a string so the
    /// downstream sanitizers/dispatchers always see the same shape.
    #[serde(default = "empty_object_string", deserialize_with = "coerce_arguments")]
    pub arguments: String,
}

fn empty_object_string() -> String {
    "{}".to_string()
}

/// Serde deserializer that accepts whatever the provider sent for `arguments`
/// and always produces a `String` field:
/// - a string value is taken verbatim;
/// - a non-string value (e.g. an object) is re-serialized back to a string;
/// - a missing value or null becomes "{}".
fn coerce_arguments<'de, D>(de: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<Value>::deserialize(de)?.unwrap_or(Value::Null);
    match v {
        Value::String(s) => Ok(s),
        Value::Null => Ok("{}".to_string()),
        other => Ok(match serde_json::to_string(&other) {
            Ok(s) => s,
            // Fallback for un-serializable values (shouldn't happen, but never
            // make deserialization itself fail).
            Err(_) => "{}".to_string(),
        }),
    }
}

/// Serde deserializer for assistant `content`. Accepts: a string (kept verbatim);
/// a multimodal array (text parts joined with a newline, image parts become
/// a placeholder so they're not dropped silently); null/missing (None).
fn coerce_optional_text<'de, D>(de: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<Value>::deserialize(de)?;
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(Value::Array(arr)) => {
            let mut out = String::new();
            for part in arr {
                let text = match part.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                    "text" => part.get("text").and_then(|t| t.as_str()).unwrap_or(""),
                    "image_url" => "[image]",
                    _ => "",
                };
                if text.is_empty() {
                    continue;
                }
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
            Ok(Some(out))
        }
        Some(other) => Ok(Some(other.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Constructors — ergonomic Message builders
// ---------------------------------------------------------------------------

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Message::System {
            name: None,
            content: Content::Text(content.into()),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Message::User {
            name: None,
            content: Content::Text(content.into()),
        }
    }

    pub fn user_multimodal(parts: Vec<ContentPart>) -> Self {
        Message::User {
            name: None,
            content: Content::Multimodal(parts),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Message::Assistant {
            name: None,
            content: Some(content.into()),
            thinking: None,
            tool_calls: None,
        }
    }

    pub fn assistant_tool_calls(calls: Vec<ToolCall>) -> Self {
        Message::Assistant {
            name: None,
            content: None,
            thinking: None,
            tool_calls: Some(calls),
        }
    }

    pub fn tool(call_id: impl Into<String>, result: impl Into<String>) -> Self {
        Message::Tool {
            tool_call_id: call_id.into(),
            name: None,
            content: result.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Accessors
// ---------------------------------------------------------------------------

impl Message {
    /// The role string: "system" | "user" | "assistant" | "tool".
    pub fn role(&self) -> &'static str {
        match self {
            Message::System { .. } => "system",
            Message::User { .. } => "user",
            Message::Assistant { .. } => "assistant",
            Message::Tool { .. } => "tool",
        }
    }

    /// Plain-text content, if this message has exactly a string content.
    pub fn content_text(&self) -> Option<&str> {
        match self {
            Message::System { content, .. } | Message::User { content, .. } => match content {
                Content::Text(s) => Some(s.as_str()),
                Content::Multimodal(_) => None,
            },
            Message::Assistant { content, .. } => content.as_deref(),
            Message::Tool { content, .. } => Some(content.as_str()),
        }
    }

    /// Multimodal parts, if any.
    pub fn content_parts(&self) -> Option<&[ContentPart]> {
        match self {
            Message::System {
                content: Content::Multimodal(p),
                ..
            }
            | Message::User {
                content: Content::Multimodal(p),
                ..
            } => Some(p.as_slice()),
            _ => None,
        }
    }

    /// Tool calls, if this is an assistant message with tool_calls.
    pub fn tool_calls(&self) -> Option<&[ToolCall]> {
        match self {
            Message::Assistant {
                tool_calls: Some(ref tc),
                ..
            } => Some(tc.as_slice()),
            _ => None,
        }
    }

    /// Assistant thinking content, if any.
    pub fn thinking(&self) -> Option<&str> {
        match self {
            Message::Assistant {
                thinking: Some(ref t),
                ..
            } => Some(t.as_str()),
            _ => None,
        }
    }

    /// Tool result call-id, for tool messages only.
    pub fn tool_call_id(&self) -> Option<&str> {
        match self {
            Message::Tool {
                tool_call_id: ref id,
                ..
            } => Some(id.as_str()),
            _ => None,
        }
    }

    // Predicates
    pub fn is_system(&self) -> bool {
        matches!(self, Message::System { .. })
    }
    pub fn is_user(&self) -> bool {
        matches!(self, Message::User { .. })
    }
    pub fn is_assistant(&self) -> bool {
        matches!(self, Message::Assistant { .. })
    }
    pub fn is_tool(&self) -> bool {
        matches!(self, Message::Tool { .. })
    }
    pub fn has_tool_calls(&self) -> bool {
        matches!(
            self,
            Message::Assistant {
                tool_calls: Some(_),
                ..
            }
        )
    }
}

// ---------------------------------------------------------------------------
// Conversion: Message ↔ serde_json::Value (the old format, for gradual
// migration and backwards-compat).
// ---------------------------------------------------------------------------

impl From<&Message> for Value {
    fn from(msg: &Message) -> Value {
        // We literally re-serialise the Message — because the serde attributes
        // are designed to match the old JSON format exactly, this produces the
        // same JSON that the old `Vec<Value>` pipeline did.
        serde_json::to_value(msg).unwrap_or(Value::Null)
    }
}

impl TryFrom<&Value> for Message {
    type Error = String;
    fn try_from(v: &Value) -> Result<Self, Self::Error> {
        serde_json::from_value(v.clone()).map_err(|e| format!("invalid message: {e}"))
    }
}

// ---------------------------------------------------------------------------
// Batch conversion helpers
// ---------------------------------------------------------------------------

/// Convert a slice of Messages into the old `Vec<Value>` format.  Useful during
/// migration when a function still expects `&[Value]`.
pub fn to_values(messages: &[Message]) -> Vec<Value> {
    messages.iter().map(Value::from).collect()
}

/// Try to parse a slice of JSON Values into Messages.  Returns an error on the
/// first malformed entry.
#[allow(dead_code)]
pub fn from_values(values: &[Value]) -> Result<Vec<Message>, String> {
    values.iter().map(Message::try_from).collect()
}

/// Best-effort variant: silently skips any malformed entry instead of failing
/// the whole batch.  Use when the source may contain hand-edited or externally-
/// sourced messages where a single bad entry shouldn't drop the entire history.
#[allow(dead_code)]
pub fn from_values_best_effort(values: &[Value]) -> Vec<Message> {
    values
        .iter()
        .filter_map(|v| Message::try_from(v).ok())
        .collect()
}

// ---------------------------------------------------------------------------
// Wire-format helpers (protocol-specific → placed here so provider.rs doesn't
// have to reach into raw JSON for message fields).
// ---------------------------------------------------------------------------

impl Message {
    /// Build the top-level `messages` array for an **OpenAI `/v1/chat/completions`**
    /// request body directly from this slice of Messages — no intermediate
    /// translation step.
    pub fn to_openai_messages(messages: &[Self]) -> Vec<Value> {
        to_values(messages)
    }

    /// Build the `tools` array for an **OpenAI `/v1/chat/completions`** request
    /// body from a list of tool definitions (still in OpenAI function-calling
    /// shape — the canonical tool-definition format is provider-agnostic for
    /// now since both OpenAI and the Anthropic converter read the same schema).
    pub fn to_openai_tools(defs: &[Value]) -> Vec<Value> {
        defs.to_vec()
    }
}

// ---------------------------------------------------------------------------
// Anthropic wire-format (native, not a JSON→JSON translator — reads Message
// fields directly via pattern matching).
// ---------------------------------------------------------------------------

/// Build an Anthropic `/v1/messages` request body from Messages.  This
/// REPLACES the old `build_anthropic_request(msgs: &[Value])` — it works on
/// typed Messages instead of opaque JSON, so there are no `.get("role")` /
/// `.get("content")` string-key lookups to silently return the wrong field.
///
/// Prompt-cache breakpoints (`cache_control: ephemeral`):
/// - Standing system prompt (leading system messages only) gets an explicit
///   breakpoint — stable across turns within a session.
/// - System messages that appear AFTER conversation content (relevant-memory /
///   work-state tails) are emitted as a final **user** message so they never
///   sit under the system breakpoint (that would bust the cache every turn).
/// - A rolling breakpoint is placed on the last content block of the last
///   **persisted** message (not on the transient tail).
pub fn build_anthropic_request(
    messages: &[Message],
    tools: &[Value],
    reasoning_effort: &str,
    thinking_levels: &[String],
    max_tokens: u32,
) -> Value {
    let mut system_parts: Vec<String> = Vec::new();
    let mut trailing_transient: Vec<String> = Vec::new();
    let mut out: Vec<Value> = Vec::new();
    let mut seen_non_system = false;

    for m in messages {
        match m {
            Message::System { content, .. } => {
                if !seen_non_system {
                    push_content(content, &mut system_parts);
                } else {
                    // Transient tails pushed after conversation content — keep
                    // them out of the cached system prefix.
                    push_content(content, &mut trailing_transient);
                }
            }
            Message::User { content, .. } => {
                seen_non_system = true;
                push_or_merge_anth(&mut out, "user", content_to_blocks(content));
            }
            Message::Assistant {
                content: ref text,
                thinking: _,
                tool_calls,
                ..
            } => {
                seen_non_system = true;
                let mut blocks = Vec::new();
                if let Some(t) = text {
                    if !t.is_empty() {
                        blocks.push(json!({"type": "text", "text": t}));
                    }
                }
                // reasoning_content is intentionally dropped: Anthropic rejects raw
                // thinking blocks without signatures (it would 400).
                if let Some(ref calls) = tool_calls {
                    for tc in calls {
                        let input: Value = serde_json::from_str(&tc.function.arguments)
                            .unwrap_or_else(|_| json!({}));
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.function.name,
                            "input": input,
                        }));
                    }
                }
                if blocks.is_empty() {
                    blocks.push(json!({"type": "text", "text": ""}));
                }
                push_or_merge_anth(&mut out, "assistant", blocks);
            }
            Message::Tool {
                ref tool_call_id,
                ref content,
                ..
            } => {
                seen_non_system = true;
                push_or_merge_anth(
                    &mut out,
                    "user",
                    vec![json!({
                        "type": "tool_result",
                        "tool_use_id": tool_call_id,
                        "content": content,
                    })],
                );
            }
        }
    }

    // Rolling breakpoint on the last persisted message (before any transient
    // tail we may append below). Anthropic's lookback is 20 blocks.
    if let Some(last) = out.last_mut() {
        if let Some(arr) = last.get_mut("content").and_then(|c| c.as_array_mut()) {
            if let Some(block) = arr.last_mut() {
                if block.is_object() {
                    block
                        .as_object_mut()
                        .unwrap()
                        .insert("cache_control".into(), json!({"type": "ephemeral"}));
                }
            }
        }
    }

    // Transient tails as a final user message — no cache_control (changes
    // every turn; must not be the automatic/explicit breakpoint).
    if !trailing_transient.is_empty() {
        out.push(json!({
            "role": "user",
            "content": [{
                "type": "text",
                "text": trailing_transient.join("\n\n"),
            }]
        }));
    }

    let mut body = serde_json::Map::new();
    // model is set by the caller (stream_turn_anthropic appends it)
    body.insert("max_tokens".into(), json!(max_tokens));
    if !system_parts.is_empty() {
        // Explicit system breakpoint: standing prompt is large enough to clear
        // Anthropic's min-token threshold and is stable within a session.
        body.insert(
            "system".into(),
            json!([{
                "type": "text",
                "text": system_parts.join("\n\n"),
                "cache_control": {"type": "ephemeral"}
            }]),
        );
    }
    if !out.is_empty() {
        body.insert("messages".into(), Value::Array(out));
    }
    if !tools.is_empty() {
        let mut atools = anthropic_tools_from_defs(tools);
        // Cache tools+system as a shared prefix: breakpoint on the last tool.
        if let Some(last) = atools.last_mut() {
            if let Some(obj) = last.as_object_mut() {
                obj.insert("cache_control".into(), json!({"type": "ephemeral"}));
            }
        }
        body.insert("tools".into(), Value::Array(atools));
        body.insert("tool_choice".into(), json!({"type": "auto"}));
    }

    if !thinking_levels.is_empty() {
        let wants = !matches!(
            reasoning_effort.to_ascii_lowercase().as_str(),
            "" | "none" | "minimal" | "off"
        );
        if wants {
            let resolved = resolve_effort_local(reasoning_effort, thinking_levels);
            if let Some(budget) = anthropic_thinking_budget(&resolved, max_tokens) {
                body.insert(
                    "thinking".into(),
                    json!({"type": "enabled", "budget_tokens": budget}),
                );
            }
        }
    }

    Value::Object(body)
}

// ---- private helpers for the Anthropic builder ----

fn push_content(content: &Content, parts: &mut Vec<String>) {
    match content {
        Content::Text(s) => {
            if !s.is_empty() {
                parts.push(s.clone());
            }
        }
        Content::Multimodal(arr) => {
            for p in arr {
                if let ContentPart::Text { text } = p {
                    if !text.is_empty() {
                        parts.push(text.clone());
                    }
                }
            }
        }
    }
}

fn content_to_blocks(content: &Content) -> Vec<Value> {
    match content {
        Content::Text(s) => vec![json!({"type": "text", "text": s})],
        Content::Multimodal(arr) => arr
            .iter()
            .map(|p| match p {
                ContentPart::Text { text } => json!({"type": "text", "text": text}),
                ContentPart::Image { image_url } => {
                    anthropic_image_block(&image_url.url, image_url.detail.as_deref())
                }
            })
            .collect(),
    }
}

/// Build an Anthropic `image` block from an OpenAI `image_url.url`. Supports
/// `data:<media>;base64,<data>` (-> base64 source) and plain URLs (-> url source).
/// The optional `detail` is forwarded as a `detail` field on the source when the
/// url source path is taken (Anthropic image blocks accept a `detail` hint).
fn anthropic_image_block(url: &str, detail: Option<&str>) -> Value {
    if let Some(rest) = url.strip_prefix("data:") {
        if let Some((meta, data)) = rest.split_once(',') {
            let media = meta.split(';').next().unwrap_or("image/png");
            return json!({
                "type": "image",
                "source": { "type": "base64", "media_type": media, "data": data }
            });
        }
    }
    let mut img = json!({"type": "image", "source": {
        "type": "url",
        "url": url,
    }});
    if let Some(d) = detail {
        img["source"]["detail"] = json!(d);
    }
    img
}

fn push_or_merge_anth(out: &mut Vec<Value>, role: &str, blocks: Vec<Value>) {
    if let Some(last) = out.last_mut() {
        if last.get("role").and_then(|v| v.as_str()) == Some(role) {
            if let Some(arr) = last.get_mut("content").and_then(|c| c.as_array_mut()) {
                arr.extend(blocks);
                return;
            }
        }
    }
    out.push(json!({"role": role, "content": blocks}));
}

/// Convert OpenAI function-calling tool defs to Anthropic `input_schema` tools.
fn anthropic_tools_from_defs(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|t| {
            let f = t.get("function")?;
            let name = f.get("name")?.as_str()?;
            let description = f.get("description").and_then(|v| v.as_str()).unwrap_or("");
            let schema = f.get("parameters").cloned().unwrap_or_else(|| json!({}));
            Some(json!({
                "name": name,
                "description": description,
                "input_schema": schema,
            }))
        })
        .collect()
}

/// Map a reasoning effort to an Anthropic extended-thinking token budget.
fn anthropic_thinking_budget(effort: &str, max_tokens: u32) -> Option<u32> {
    let base: u32 = match effort.to_ascii_lowercase().as_str() {
        "low" | "minimal" => 4096,
        "medium" => 12288,
        "high" | "max" => 24576,
        _ => return None,
    };
    let budget = base.min(max_tokens.saturating_sub(1024));
    if budget < 1024 {
        return None;
    }
    Some(budget)
}

/// Resolve a reasoning effort against the model's supported levels.
fn resolve_effort_local(requested: &str, levels: &[String]) -> String {
    // Re-implemented from provider.rs::resolve_effort to avoid a circular dep.
    // Once message.rs is the canonical home, provider.rs can re-export or call
    // this instead.
    let r = requested.to_ascii_lowercase();
    if levels.is_empty() {
        return requested.to_string();
    }
    if r == "none" || r == "minimal" || r == "off" || requested.is_empty() {
        return requested.to_string();
    }
    let lower: Vec<String> = levels.iter().map(|l| l.to_ascii_lowercase()).collect();
    if lower.iter().any(|l| l == &r) {
        return requested.to_string();
    }
    // Clamp to closest supported level.
    if lower.len() == 1 {
        return levels[0].clone();
    }
    let prio = ["high", "max", "medium", "low"];
    for p in &prio {
        if lower.iter().any(|l| l == p) {
            return p.to_string();
        }
    }
    levels[0].clone()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_old_json_format() {
        let msgs: Vec<Message> = vec![
            Message::system("you are a helpful assistant"),
            Message::user("hello"),
            Message::Assistant {
                name: None,
                content: Some("hi there".into()),
                thinking: None,
                tool_calls: None,
            },
            Message::Tool {
                tool_call_id: "call_1".into(),
                name: None,
                content: "result".into(),
            },
        ];
        let values: Vec<Value> = msgs.iter().map(Value::from).collect();
        let roundtripped: Vec<Message> = values
            .iter()
            .map(|v| Message::try_from(v).unwrap())
            .collect();

        // Verify round-trip preserves content
        assert_eq!(roundtripped.len(), msgs.len());
        assert!(roundtripped[0].is_system());
        assert_eq!(
            roundtripped[0].content_text(),
            Some("you are a helpful assistant")
        );
        assert!(roundtripped[1].is_user());
        assert_eq!(roundtripped[1].content_text(), Some("hello"));
        assert!(roundtripped[2].is_assistant());
        assert_eq!(roundtripped[2].content_text(), Some("hi there"));
        assert!(roundtripped[3].is_tool());
        assert_eq!(roundtripped[3].content_text(), Some("result"));
        assert_eq!(roundtripped[3].tool_call_id(), Some("call_1"));
    }

    #[test]
    fn assistant_with_tool_calls_roundtrips() {
        let msg = Message::assistant_tool_calls(vec![ToolCall {
            id: "call_0".into(),
            typ: "function".into(),
            function: FunctionCall {
                name: "bash".into(),
                arguments: r#"{"command":"ls"}"#.into(),
            },
        }]);
        let v = Value::from(&msg);
        let got = Message::try_from(&v).unwrap();
        let calls = got.tool_calls().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "bash");
        assert_eq!(calls[0].function.arguments, r#"{"command":"ls"}"#);
    }

    #[test]
    fn multimodal_roundtrips() {
        let msg = Message::user_multimodal(vec![
            ContentPart::Text {
                text: "look at this".into(),
            },
            ContentPart::Image {
                image_url: ImageUrl {
                    url: "data:image/png;base64,abc".into(),
                    detail: Some("auto".into()),
                },
            },
        ]);
        let v = Value::from(&msg);
        let got = Message::try_from(&v).unwrap();
        let parts = got.content_parts().unwrap();
        assert_eq!(parts.len(), 2);
        match &parts[1] {
            ContentPart::Image { image_url } => {
                assert!(image_url.url.starts_with("data:image"));
            }
            _ => panic!("expected image"),
        }
    }

    #[test]
    fn reasoning_content_roundtrips_via_thinking_field() {
        // Old-format assistant message with "reasoning_content" must deserialize
        // to Message::thinking and re-serialize with the same key name.
        let old = json!({"role":"assistant","content":"ok","reasoning_content":"let me think..."});
        let msg = Message::try_from(&old).unwrap();
        assert!(msg.is_assistant());
        assert_eq!(msg.thinking(), Some("let me think..."));
        let v = Value::from(&msg);
        assert_eq!(v["reasoning_content"], "let me think...");
        assert_eq!(v["content"], "ok");
    }

    #[test]
    fn anthropic_request_builds_from_messages() {
        let msgs = vec![
            Message::system("you are a helpful assistant"),
            Message::user("read foo"),
            Message::assistant_tool_calls(vec![ToolCall {
                id: "call_1".into(),
                typ: "function".into(),
                function: FunctionCall {
                    name: "read_file".into(),
                    arguments: r#"{"path":"foo.rs"}"#.into(),
                },
            }]),
            Message::tool("call_1", "contents of foo"),
        ];
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a file",
                "parameters": {"type": "object", "properties": {}, "required": []}
            }
        })];
        let body = build_anthropic_request(&msgs, &tools, "none", &[], 4096);
        // System is an array of content blocks with an explicit cache breakpoint.
        assert_eq!(body["system"][0]["type"], "text");
        assert_eq!(body["system"][0]["text"], "you are a helpful assistant");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        // Last tool carries a tools-prefix breakpoint.
        assert_eq!(body["tools"][0]["cache_control"]["type"], "ephemeral");
        let messages = body["messages"].as_array().unwrap();
        // user, assistant(tool_use), user(tool_result) — roles alternate
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[2]["role"], "user");
        // Rolling breakpoint on the last persisted message (tool_result).
        assert_eq!(
            messages[2]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        // assistant content is a tool_use block
        let blocks = messages[1]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "tool_use");
        assert_eq!(blocks[0]["id"], "call_1");
        assert_eq!(blocks[0]["name"], "read_file");
        assert_eq!(blocks[0]["input"]["path"], "foo.rs");
        // tool result
        let rblocks = messages[2]["content"].as_array().unwrap();
        assert_eq!(rblocks[0]["type"], "tool_result");
        assert_eq!(rblocks[0]["tool_use_id"], "call_1");
        assert_eq!(rblocks[0]["content"], "contents of foo");
        // tools converted
        let at = body["tools"].as_array().unwrap();
        assert_eq!(at[0]["name"], "read_file");
        assert_eq!(at[0]["input_schema"]["type"], "object");
    }

    #[test]
    fn anthropic_transient_system_tail_not_under_system_breakpoint() {
        // Standing system + conversation + trailing work-state system message:
        // the tail must become a user message WITHOUT cache_control, and the
        // rolling breakpoint must land on the prior persisted message.
        let msgs = vec![
            Message::system("stable system"),
            Message::user("hello"),
            Message::assistant("hi"),
            Message::system("[WORK STATE]\ngoal: x"),
        ];
        let body = build_anthropic_request(&msgs, &[], "none", &[], 4096);
        assert_eq!(body["system"][0]["text"], "stable system");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[2]["role"], "user");
        assert!(messages[2]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("[WORK STATE]"));
        assert!(messages[2]["content"][0].get("cache_control").is_none());
        // Rolling breakpoint on the assistant reply (last persisted).
        assert_eq!(
            messages[1]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn anthropic_thinking_budget_maps_effort() {
        let budget = anthropic_thinking_budget("high", 32768);
        assert!(budget.is_some());
        assert!(budget.unwrap() >= 1024 && budget.unwrap() <= 32768);
        // "none" returns None
        assert_eq!(anthropic_thinking_budget("none", 4096), None);
    }

    #[test]
    fn anthropic_request_with_thinking() {
        let msgs = vec![Message::user("hi")];
        let body = build_anthropic_request(
            &msgs,
            &[],
            "high",
            &["low".into(), "medium".into(), "high".into()],
            32768,
        );
        let thinking = body.get("thinking").unwrap();
        assert_eq!(thinking["type"], "enabled");
        assert!(thinking["budget_tokens"].as_u64().unwrap() >= 4096);
    }

    // --- tolerant deserialization (P1 #1 / P1 #2) -----------------------

    /// A tool call whose `arguments` is a JSON **object** (not a string)
    /// must deserialize without failing; the object is coerced back to a string
    /// so downstream sanitizers/dispatchers always see the string shape.
    #[test]
    fn tool_call_object_arguments_coerced_to_string() {
        let v = json!({
            "role":"assistant",
            "tool_calls":[{
                "id":"call_1","type":"function",
                "function":{"name":"bash","arguments":{"command":"echo hi"}}
            }]
        });
        let msg = Message::try_from(&v).unwrap();
        let tc = &msg.tool_calls().unwrap()[0];
        assert_eq!(tc.function.name, "bash");
        // The object must have been serialized back into a string.
        let parsed: Value = serde_json::from_str(&tc.function.arguments).unwrap();
        assert_eq!(parsed["command"], "echo hi");
    }

    /// A tool call whose `arguments` field is missing must default to "{}"
    /// instead of failing deserialization (so the turn isn't aborted before
    /// the sanitizer can fix it).
    #[test]
    fn tool_call_missing_arguments_defaults_to_empty_object_string() {
        let v = json!({
            "role":"assistant",
            "tool_calls":[{
                "id":"call_2","type":"function",
                "function":{"name":"finish"}
            }]
        });
        let msg = Message::try_from(&v).unwrap();
        let tc = &msg.tool_calls().unwrap()[0];
        assert_eq!(tc.function.arguments, "{}");
    }

    /// An old-format assistant message whose `content` is a multimodal array
    /// (text + image parts) must deserialize into the string field without
    /// dropping the text — it joins text parts and replaces images with a
    /// placeholder instead of failing.
    #[test]
    fn assistant_array_content_coerced_to_joined_text() {
        let v = json!({
            "role":"assistant",
            "content":[
                {"type":"text","text":"hello"},
                {"type":"text","text":"world"},
                {"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}
            ]
        });
        let msg = Message::try_from(&v).unwrap();
        assert_eq!(msg.content_text(), Some("hello\nworld\n[image]"));
    }

    /// An assistant with only tool_calls (no `content` field) deserializes fine
    /// and its content is None.
    #[test]
    fn assistant_only_tool_calls_has_none_content() {
        let v = json!({
            "role":"assistant",
            "tool_calls":[{"id":"c","type":"function","function":{"name":"x","arguments":"{}"}}]
        });
        let msg = Message::try_from(&v).unwrap();
        assert!(msg.content_text().is_none());
        assert!(msg.tool_calls().is_some());
    }

    // --- Anthropic image data-URL regression guard (P2 #3) ----------------

    /// A multimodal user message with a `data:` URL image must become an
    /// Anthropic **base64** source block (not a url source) — the regression
    /// fixed in this change.
    #[test]
    fn anthropic_data_url_becomes_base64_source() {
        let msg = Message::user_multimodal(vec![
            ContentPart::Text {
                text: "look".into(),
            },
            ContentPart::Image {
                image_url: ImageUrl {
                    url: "data:image/png;base64,AAAA".into(),
                    detail: None,
                },
            },
        ]);
        let body = build_anthropic_request(&[msg], &[], "none", &[], 4096);
        let blocks = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(blocks[1]["type"], "image");
        assert_eq!(blocks[1]["source"]["type"], "base64");
        assert_eq!(blocks[1]["source"]["media_type"], "image/png");
        assert_eq!(blocks[1]["source"]["data"], "AAAA");
    }

    /// A plain-URL image (not a data URL) still routes to a url source with the
    /// detail hint preserved.
    #[test]
    fn anthropic_plain_url_image_keeps_url_source_with_detail() {
        let msg = Message::user_multimodal(vec![ContentPart::Image {
            image_url: ImageUrl {
                url: "https://example.test/cat.png".into(),
                detail: Some("high".into()),
            },
        }]);
        let body = build_anthropic_request(&[msg], &[], "none", &[], 4096);
        let blocks = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["source"]["type"], "url");
        assert_eq!(blocks[0]["source"]["url"], "https://example.test/cat.png");
        assert_eq!(blocks[0]["source"]["detail"], "high");
    }
}
