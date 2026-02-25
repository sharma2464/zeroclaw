use crate::providers::{ChatMessage, ChatResponse, ConversationMessage, ToolResultMessage};
use crate::tools::{Tool, ToolSpec};
use regex::Regex;
use serde_json::Value;
use std::fmt::Write;
use std::sync::LazyLock;

#[derive(Debug, Clone)]
pub struct ParsedToolCall {
    pub name: String,
    pub arguments: Value,
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToolExecutionResult {
    pub name: String,
    pub output: String,
    pub success: bool,
    pub tool_call_id: Option<String>,
}

pub trait ToolDispatcher: Send + Sync {
    fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ParsedToolCall>);
    fn format_results(&self, results: &[ToolExecutionResult]) -> ConversationMessage;
    fn prompt_instructions(&self, tools: &[Box<dyn Tool>]) -> String;
    fn to_provider_messages(&self, history: &[ConversationMessage]) -> Vec<ChatMessage>;
    fn should_send_tool_specs(&self) -> bool;
}

#[derive(Default)]
pub struct XmlToolDispatcher;

const TOOL_CALL_OPEN_TAGS: [&str; 4] = ["<tool_call", "<toolcall", "<tool-call", "<invoke"];

fn map_glm_tool_alias(tool_name: &str) -> &str {
    match tool_name {
        "browser_open" | "browser" | "web_search" | "shell" | "bash" => "shell",
        "http_request" | "http" => "http_request",
        _ => tool_name,
    }
}

fn build_curl_command(url: &str) -> Option<String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }

    if url.chars().any(char::is_whitespace) {
        return None;
    }

    let escaped = url.replace('\'', r#"'\\''"#);
    Some(format!("curl -s '{}'", escaped))
}

fn parse_glm_style_tool_calls(text: &str) -> Vec<(String, Value, Option<String>)> {
    let mut calls = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Format: tool_name/param>value or tool_name/{json}
        if let Some(pos) = line.find('/') {
            let tool_part = &line[..pos];
            let rest = &line[pos + 1..];

            if tool_part.chars().all(|c| c.is_alphanumeric() || c == '_') {
                let tool_name = map_glm_tool_alias(tool_part);

                if let Some(gt_pos) = rest.find('>') {
                    let param_name = rest[..gt_pos].trim();
                    let value = rest[gt_pos + 1..].trim();

                    let arguments = match tool_name {
                        "shell" => {
                            if param_name == "url" {
                                let Some(command) = build_curl_command(value) else {
                                    continue;
                                };
                                serde_json::json!({"command": command})
                            } else if value.starts_with("http://") || value.starts_with("https://")
                            {
                                if let Some(command) = build_curl_command(value) {
                                    serde_json::json!({"command": command})
                                } else {
                                    serde_json::json!({"command": value})
                                }
                            } else {
                                serde_json::json!({"command": value})
                            }
                        }
                        "http_request" => {
                            serde_json::json!({"url": value, "method": "GET"})
                        }
                        _ => serde_json::json!({param_name: value}),
                    };

                    calls.push((tool_name.to_string(), arguments, Some(line.to_string())));
                    continue;
                }

                if rest.starts_with('{') {
                    if let Ok(json_args) = serde_json::from_str::<Value>(rest) {
                        calls.push((tool_name.to_string(), json_args, Some(line.to_string())));
                    }
                }
            }
        }

        // Plain URL
        if let Some(command) = build_curl_command(line) {
            calls.push((
                "shell".to_string(),
                serde_json::json!({"command": command}),
                Some(line.to_string()),
            ));
        }
    }

    calls
}

fn find_first_tag<'a>(haystack: &str, tags: &'a [&'a str]) -> Option<(usize, &'a str)> {
    let mut best: Option<(usize, &str)> = None;

    for &tag in tags {
        // Look for the tag followed by either a space or a closing bracket
        let mut start_idx = 0;
        while let Some(idx) = haystack[start_idx..].find(tag) {
            let actual_idx = start_idx + idx;
            let after_tag = &haystack[actual_idx + tag.len()..];
            if after_tag.starts_with('>') || after_tag.starts_with(char::is_whitespace) {
                if best.is_none() || actual_idx < best.unwrap().0 {
                    best = Some((actual_idx, tag));
                }
                break;
            }
            start_idx = actual_idx + 1;
        }
    }
    best
}

fn matching_tool_call_close_tag(open_tag: &str) -> Option<&'static str> {
    if open_tag.starts_with("<tool_call") {
        Some("</tool_call>")
    } else if open_tag.starts_with("<toolcall") {
        Some("</toolcall>")
    } else if open_tag.starts_with("<tool-call") {
        Some("</tool-call>")
    } else if open_tag.starts_with("<invoke") {
        Some("</invoke>")
    } else {
        None
    }
}

fn find_tag_end(haystack: &str, start_idx: usize) -> Option<usize> {
    haystack[start_idx..].find('>').map(|idx| start_idx + idx + 1)
}

fn parse_arguments_value(raw: Option<&Value>) -> Value {
    match raw {
        Some(Value::String(s)) => serde_json::from_str::<Value>(s)
            .unwrap_or_else(|_| Value::Object(serde_json::Map::new())),
        Some(value) => value.clone(),
        None => Value::Object(serde_json::Map::new()),
    }
}

fn parse_tool_call_value(value: &Value) -> Option<ParsedToolCall> {
    if let Some(function) = value.get("function") {
        let name = function
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if !name.is_empty() {
            let arguments = parse_arguments_value(function.get("arguments"));
            return Some(ParsedToolCall {
                name,
                arguments,
                tool_call_id: None,
            });
        }
    }

    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if name.is_empty() {
        return None;
    }

    let arguments = parse_arguments_value(value.get("arguments"));
    Some(ParsedToolCall {
        name,
        arguments,
        tool_call_id: None,
    })
}

fn parse_tool_calls_from_json_value(value: &Value) -> Vec<ParsedToolCall> {
    let mut calls = Vec::new();

    if let Some(tool_calls) = value.get("tool_calls").and_then(|v| v.as_array()) {
        for call in tool_calls {
            if let Some(parsed) = parse_tool_call_value(call) {
                calls.push(parsed);
            }
        }

        if !calls.is_empty() {
            return calls;
        }
    }

    if let Some(array) = value.as_array() {
        for item in array {
            if let Some(parsed) = parse_tool_call_value(item) {
                calls.push(parsed);
            }
        }
        return calls;
    }

    if let Some(parsed) = parse_tool_call_value(value) {
        calls.push(parsed);
    }

    calls
}

fn extract_json_values(input: &str) -> Vec<Value> {
    let mut values = Vec::new();
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return values;
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        values.push(value);
        return values;
    }

    let char_positions: Vec<(usize, char)> = trimmed.char_indices().collect();
    let mut idx = 0;
    while idx < char_positions.len() {
        let (byte_idx, ch) = char_positions[idx];
        if ch == '{' || ch == '[' {
            let slice = &trimmed[byte_idx..];
            let mut stream = serde_json::Deserializer::from_str(slice).into_iter::<Value>();
            if let Some(Ok(value)) = stream.next() {
                let consumed = stream.byte_offset();
                if consumed > 0 {
                    values.push(value);
                    let next_byte = byte_idx + consumed;
                    while idx < char_positions.len() && char_positions[idx].0 < next_byte {
                        idx += 1;
                    }
                    continue;
                }
            }
        }
        idx += 1;
    }

    values
}

fn find_json_end(input: &str) -> Option<usize> {
    let trimmed = input.trim_start();
    let offset = input.len() - trimmed.len();

    if !trimmed.starts_with('{') {
        return None;
    }

    let mut depth = 0;
    let mut in_string = false;
    let mut escape_next = false;

    for (i, ch) in trimmed.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }

        match ch {
            '\\' if in_string => escape_next = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(offset + i + ch.len_utf8());
                }
            }
            _ => {}
        }
    }

    None
}

fn extract_first_json_value_with_end(input: &str) -> Option<(Value, usize)> {
    let trimmed = input.trim_start();
    let trim_offset = input.len().saturating_sub(trimmed.len());

    for (byte_idx, ch) in trimmed.char_indices() {
        if ch != '{' && ch != '[' {
            continue;
        }

        let slice = &trimmed[byte_idx..];
        let mut stream = serde_json::Deserializer::from_str(slice).into_iter::<Value>();
        if let Some(Ok(value)) = stream.next() {
            let consumed = stream.byte_offset();
            if consumed > 0 {
                return Some((value, trim_offset + byte_idx + consumed));
            }
        }
    }

    None
}

fn strip_leading_close_tags(mut input: &str) -> &str {
    loop {
        let trimmed = input.trim_start();
        if !trimmed.starts_with("</") {
            return trimmed;
        }

        let Some(close_end) = trimmed.find('>') else {
            return "";
        };
        input = &trimmed[close_end + 1..];
    }
}

static MD_TOOL_CALL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?s)```(?:tool[_-]?call|invoke)\s*\n(.*?)(?:```|</tool[_-]?call>|</toolcall>|</invoke>)",
    )
    .unwrap()
});

fn matching_tool_call_close_re(open_tag: &str) -> Option<Regex> {
    if open_tag.starts_with("<tool_call") {
        Some(Regex::new(r"(?i)</\s*tool_call\s*>").unwrap())
    } else if open_tag.starts_with("<toolcall") {
        Some(Regex::new(r"(?i)</\s*toolcall\s*>").unwrap())
    } else if open_tag.starts_with("<tool-call") {
        Some(Regex::new(r"(?i)</\s*tool-call\s*>").unwrap())
    } else if open_tag.starts_with("<invoke") {
        Some(Regex::new(r"(?i)</\s*invoke\s*>").unwrap())
    } else {
        None
    }
}

impl XmlToolDispatcher {
    pub(crate) fn parse_xml_tool_calls(response: &str) -> (String, Vec<ParsedToolCall>) {
        let mut text_parts = Vec::new();
        let mut calls = Vec::new();
        let mut remaining = response;

        // Try OpenAI-style JSON first
        if let Ok(json_value) = serde_json::from_str::<Value>(response.trim()) {
            let json_calls = parse_tool_calls_from_json_value(&json_value);
            if !json_calls.is_empty() {
                if let Some(content) = json_value.get("content").and_then(|v| v.as_str()) {
                    if !content.trim().is_empty() {
                        text_parts.push(content.trim().to_string());
                    }
                }
                return (text_parts.join("\n"), json_calls);
            }
        }

        while let Some((start, _prefix)) = find_first_tag(remaining, &TOOL_CALL_OPEN_TAGS) {
            let before = &remaining[..start];
            if !before.trim().is_empty() {
                text_parts.push(before.trim().to_string());
            }

            let Some(tag_end_idx) = find_tag_end(remaining, start) else {
                break;
            };

            let open_tag = &remaining[start..tag_end_idx];
            let Some(close_re) = matching_tool_call_close_re(open_tag) else {
                break;
            };

            let after_open = &remaining[tag_end_idx..];
            if let Some(mat) = close_re.find(after_open) {
                let close_start = mat.start();
                let close_end = mat.end();
                let inner = &after_open[..close_start];
                let mut parsed_any = false;
                let json_values = extract_json_values(inner);
                for value in json_values {
                    let parsed_calls = parse_tool_calls_from_json_value(&value);
                    if !parsed_calls.is_empty() {
                        parsed_any = true;
                        calls.extend(parsed_calls);
                    }
                }

                if !parsed_any {
                    tracing::warn!(
                        "Malformed <tool_call> JSON: expected tool-call object in tag body"
                    );
                }

                remaining = &after_open[close_end..];
            } else {
                if let Some(json_end) = find_json_end(after_open) {
                    if let Ok(value) = serde_json::from_str::<Value>(&after_open[..json_end]) {
                        let parsed_calls = parse_tool_calls_from_json_value(&value);
                        if !parsed_calls.is_empty() {
                            calls.extend(parsed_calls);
                            remaining = strip_leading_close_tags(&after_open[json_end..]);
                            continue;
                        }
                    }
                }

                if let Some((value, consumed_end)) = extract_first_json_value_with_end(after_open) {
                    let parsed_calls = parse_tool_calls_from_json_value(&value);
                    if !parsed_calls.is_empty() {
                        calls.extend(parsed_calls);
                        remaining = strip_leading_close_tags(&after_open[consumed_end..]);
                        continue;
                    }
                }

                remaining = &remaining[start..];
                break;
            }
        }

        // Markdown blocks
        if calls.is_empty() {
            let mut md_text_parts: Vec<String> = Vec::new();
            let mut last_end = 0;

            for cap in MD_TOOL_CALL_RE.captures_iter(response) {
                let full_match = cap.get(0).unwrap();
                let before = &response[last_end..full_match.start()];
                if !before.trim().is_empty() {
                    md_text_parts.push(before.trim().to_string());
                }
                let inner = &cap[1];
                let json_values = extract_json_values(inner);
                for value in json_values {
                    let parsed_calls = parse_tool_calls_from_json_value(&value);
                    calls.extend(parsed_calls);
                }
                last_end = full_match.end();
            }

            if !calls.is_empty() {
                let after = &response[last_end..];
                if !after.trim().is_empty() {
                    md_text_parts.push(after.trim().to_string());
                }
                text_parts = md_text_parts;
                remaining = "";
            }
        }

        // GLM-style tool calls (browser_open/url>https://..., shell/command>ls, etc.)
        if calls.is_empty() {
            let glm_calls = parse_glm_style_tool_calls(remaining);
            if !glm_calls.is_empty() {
                let mut cleaned_text = remaining.to_string();
                for (name, args, raw) in &glm_calls {
                    calls.push(ParsedToolCall {
                        name: name.clone(),
                        arguments: args.clone(),
                        tool_call_id: None,
                    });
                    if let Some(r) = raw {
                        cleaned_text = cleaned_text.replace(r, "");
                    }
                }
                if !cleaned_text.trim().is_empty() {
                    text_parts.push(cleaned_text.trim().to_string());
                }
                remaining = "";
            }
        }

        if !remaining.trim().is_empty() {
            text_parts.push(remaining.trim().to_string());
        }

        (text_parts.join("\n"), calls)
    }

    pub fn tool_specs(tools: &[Box<dyn Tool>]) -> Vec<ToolSpec> {
        tools.iter().map(|tool| tool.spec()).collect()
    }
}

impl ToolDispatcher for XmlToolDispatcher {
    fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ParsedToolCall>) {
        let text = response.text_or_empty();
        Self::parse_xml_tool_calls(text)
    }

    fn format_results(&self, results: &[ToolExecutionResult]) -> ConversationMessage {
        let mut content = String::new();
        for result in results {
            let status = if result.success { "ok" } else { "error" };
            let _ = writeln!(
                content,
                "<tool_result name=\"{}\" status=\"{}\">\n{}\n</tool_result>",
                result.name, status, result.output
            );
        }
        ConversationMessage::Chat(ChatMessage::user(format!("[Tool results]\n{content}")))
    }

    fn prompt_instructions(&self, tools: &[Box<dyn Tool>]) -> String {
        let mut instructions = String::new();
        instructions.push_str("## Tool Use Protocol\n\n");
        instructions
            .push_str("To use a tool, wrap a JSON object in <tool_call></tool_call> tags:\n\n");
        instructions.push_str(
            "```\n<tool_call>\n{\"name\": \"tool_name\", \"arguments\": {\"param\": \"value\"}}\n</tool_call>\n```\n\n",
        );
        instructions.push_str("### Available Tools\n\n");

        for tool in tools {
            let _ = writeln!(
                instructions,
                "- **{}**: {}\n  Parameters: `{}`",
                tool.name(),
                tool.description(),
                tool.parameters_schema()
            );
        }

        instructions
    }

    fn to_provider_messages(&self, history: &[ConversationMessage]) -> Vec<ChatMessage> {
        history
            .iter()
            .flat_map(|msg| match msg {
                ConversationMessage::Chat(chat) => vec![chat.clone()],
                ConversationMessage::AssistantToolCalls { text, .. } => {
                    vec![ChatMessage::assistant(text.clone().unwrap_or_default())]
                }
                ConversationMessage::ToolResults(results) => {
                    let mut content = String::new();
                    for result in results {
                        let _ = writeln!(
                            content,
                            "<tool_result id=\"{}\">\n{}\n</tool_result>",
                            result.tool_call_id, result.content
                        );
                    }
                    vec![ChatMessage::user(format!("[Tool results]\n{content}"))]
                }
            })
            .collect()
    }

    fn should_send_tool_specs(&self) -> bool {
        false
    }
}

pub struct NativeToolDispatcher;

impl ToolDispatcher for NativeToolDispatcher {
    fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ParsedToolCall>) {
        let text = response.text.clone().unwrap_or_default();
        let calls = response
            .tool_calls
            .iter()
            .map(|tc| ParsedToolCall {
                name: tc.name.clone(),
                arguments: serde_json::from_str(&tc.arguments).unwrap_or_else(|e| {
                    tracing::warn!(
                        tool = %tc.name,
                        error = %e,
                        "Failed to parse native tool call arguments as JSON; defaulting to empty object"
                    );
                    Value::Object(serde_json::Map::new())
                }),
                tool_call_id: Some(tc.id.clone()),
            })
            .collect();
        (text, calls)
    }

    fn format_results(&self, results: &[ToolExecutionResult]) -> ConversationMessage {
        let messages = results
            .iter()
            .map(|result| ToolResultMessage {
                tool_call_id: result
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                content: result.output.clone(),
            })
            .collect();
        ConversationMessage::ToolResults(messages)
    }

    fn prompt_instructions(&self, _tools: &[Box<dyn Tool>]) -> String {
        String::new()
    }

    fn to_provider_messages(&self, history: &[ConversationMessage]) -> Vec<ChatMessage> {
        history
            .iter()
            .flat_map(|msg| match msg {
                ConversationMessage::Chat(chat) => vec![chat.clone()],
                ConversationMessage::AssistantToolCalls {
                    text,
                    tool_calls,
                    reasoning_content,
                } => {
                    let mut payload = serde_json::json!({
                        "content": text,
                        "tool_calls": tool_calls,
                    });
                    if let Some(rc) = reasoning_content {
                        payload["reasoning_content"] = serde_json::json!(rc);
                    }
                    vec![ChatMessage::assistant(payload.to_string())]
                }
                ConversationMessage::ToolResults(results) => results
                    .iter()
                    .map(|result| {
                        ChatMessage::tool(
                            serde_json::json!({
                                "tool_call_id": result.tool_call_id,
                                "content": result.content,
                            })
                            .to_string(),
                        )
                    })
                    .collect(),
            })
            .collect()
    }

    fn should_send_tool_specs(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_dispatcher_parses_tool_calls() {
        let response = ChatResponse {
            text: Some(
                "Checking\n<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool_call>"
                    .into(),
            ),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        };
        let dispatcher = XmlToolDispatcher;
        let (_, calls) = dispatcher.parse_response(&response);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell");
    }

    #[test]
    fn xml_dispatcher_handles_whitespace_in_tags() {
        let response = ChatResponse {
            text: Some(
                "<tool_call >{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool_call>"
                    .into(),
            ),
            tool_calls: vec![],
        };
        let dispatcher = XmlToolDispatcher;
        let (_, calls) = dispatcher.parse_response(&response);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell");
    }

    #[test]
    fn xml_dispatcher_handles_glm_style() {
        let response = ChatResponse {
            text: Some("shell/command>ls -la".into()),
            tool_calls: vec![],
        };
        let dispatcher = XmlToolDispatcher;
        let (_, calls) = dispatcher.parse_response(&response);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell");
        assert_eq!(calls[0].arguments["command"], "ls -la");
    }

    #[test]
    fn native_dispatcher_roundtrip() {
        let response = ChatResponse {
            text: Some("ok".into()),
            tool_calls: vec![crate::providers::ToolCall {
                id: "tc1".into(),
                name: "file_read".into(),
                arguments: "{\"path\":\"a.txt\"}".into(),
            }],
            usage: None,
            reasoning_content: None,
        };
        let dispatcher = NativeToolDispatcher;
        let (_, calls) = dispatcher.parse_response(&response);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_call_id.as_deref(), Some("tc1"));

        let msg = dispatcher.format_results(&[ToolExecutionResult {
            name: "file_read".into(),
            output: "hello".into(),
            success: true,
            tool_call_id: Some("tc1".into()),
        }]);
        match msg {
            ConversationMessage::ToolResults(results) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].tool_call_id, "tc1");
            }
            _ => panic!("expected tool results"),
        }
    }

    #[test]
    fn xml_format_results_contains_tool_result_tags() {
        let dispatcher = XmlToolDispatcher;
        let msg = dispatcher.format_results(&[ToolExecutionResult {
            name: "shell".into(),
            output: "ok".into(),
            success: true,
            tool_call_id: None,
        }]);
        let rendered = match msg {
            ConversationMessage::Chat(chat) => chat.content,
            _ => String::new(),
        };
        assert!(rendered.contains("<tool_result"));
        assert!(rendered.contains("shell"));
    }

    #[test]
    fn native_format_results_keeps_tool_call_id() {
        let dispatcher = NativeToolDispatcher;
        let msg = dispatcher.format_results(&[ToolExecutionResult {
            name: "shell".into(),
            output: "ok".into(),
            success: true,
            tool_call_id: Some("tc-1".into()),
        }]);

        match msg {
            ConversationMessage::ToolResults(results) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].tool_call_id, "tc-1");
            }
            _ => panic!("expected ToolResults variant"),
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // reasoning_content pass-through tests
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn native_to_provider_messages_includes_reasoning_content() {
        let dispatcher = NativeToolDispatcher;
        let history = vec![ConversationMessage::AssistantToolCalls {
            text: Some("answer".into()),
            tool_calls: vec![crate::providers::ToolCall {
                id: "tc_1".into(),
                name: "shell".into(),
                arguments: "{}".into(),
            }],
            reasoning_content: Some("thinking step".into()),
        }];

        let messages = dispatcher.to_provider_messages(&history);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "assistant");

        let payload: serde_json::Value = serde_json::from_str(&messages[0].content).unwrap();
        assert_eq!(payload["reasoning_content"].as_str(), Some("thinking step"));
        assert_eq!(payload["content"].as_str(), Some("answer"));
        assert!(payload["tool_calls"].is_array());
    }

    #[test]
    fn native_to_provider_messages_omits_reasoning_content_when_none() {
        let dispatcher = NativeToolDispatcher;
        let history = vec![ConversationMessage::AssistantToolCalls {
            text: Some("answer".into()),
            tool_calls: vec![crate::providers::ToolCall {
                id: "tc_1".into(),
                name: "shell".into(),
                arguments: "{}".into(),
            }],
            reasoning_content: None,
        }];

        let messages = dispatcher.to_provider_messages(&history);
        assert_eq!(messages.len(), 1);

        let payload: serde_json::Value = serde_json::from_str(&messages[0].content).unwrap();
        assert!(payload.get("reasoning_content").is_none());
    }

    #[test]
    fn xml_to_provider_messages_ignores_reasoning_content() {
        let dispatcher = XmlToolDispatcher;
        let history = vec![ConversationMessage::AssistantToolCalls {
            text: Some("answer".into()),
            tool_calls: vec![crate::providers::ToolCall {
                id: "tc_1".into(),
                name: "shell".into(),
                arguments: "{}".into(),
            }],
            reasoning_content: Some("should be ignored".into()),
        }];

        let messages = dispatcher.to_provider_messages(&history);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "assistant");
        // XmlToolDispatcher returns text only, not JSON payload
        assert_eq!(messages[0].content, "answer");
        assert!(!messages[0].content.contains("reasoning_content"));
    }
}
