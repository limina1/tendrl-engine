//! Chat state management for LLM conversations
//!
//! Pure state logic for chat fragments, edit mode, context injection,
//! and message serialization. No IO — the async wiring lives in the
//! tree command/engine layer.

use crate::llm::{AgentMessage, ContentBlock};
use crate::publication::NAddr;
use std::collections::HashSet;

/// Role of a chat message
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

impl ChatRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChatRole::System => "system",
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "system" => Some(ChatRole::System),
            "user" => Some(ChatRole::User),
            "assistant" => Some(ChatRole::Assistant),
            _ => None,
        }
    }
}

/// A single fragment in the conversation.
///
/// `blocks` carries structured agent content (text / thinking / tool_use /
/// tool_result) when this fragment came from the tool-calling loop; it is
/// `None` for plain text fragments. A `User` fragment whose `blocks` hold
/// `tool_result`s represents the results answering the preceding assistant
/// turn (Anthropic puts tool results in a user message).
#[derive(Debug, Clone)]
pub struct ChatFragment {
    pub role: ChatRole,
    pub content: String,
    pub id: usize,
    pub blocks: Option<Vec<ContentBlock>>,
}

/// A note injected as context for the LLM
#[derive(Debug, Clone)]
pub struct InjectedNote {
    pub addr: Option<NAddr>,
    pub title: String,
    pub content: String,
}

/// A message ready to send to an LLM provider
#[derive(Debug, Clone)]
pub struct LLMMessage {
    pub role: ChatRole,
    pub content: String,
}

/// State for a chat conversation
pub struct ChatState {
    pub fragments: Vec<ChatFragment>,
    pub selected: HashSet<usize>,
    pub fragment_cursor: usize,
    pub scroll: usize,
    pub input: String,
    pub input_cursor: usize,
    pub edit_mode: bool,
    pub edit_buffer: String,
    pub edit_cursor: (usize, usize),
    pub generating: bool,
    pub system_prompt: Option<String>,
    pub injected_context: Vec<InjectedNote>,
    next_id: usize,
}

impl ChatState {
    pub fn new() -> Self {
        Self {
            fragments: Vec::new(),
            selected: HashSet::new(),
            fragment_cursor: 0,
            scroll: 0,
            input: String::new(),
            input_cursor: 0,
            edit_mode: false,
            edit_buffer: String::new(),
            edit_cursor: (0, 0),
            generating: false,
            system_prompt: None,
            injected_context: Vec::new(),
            next_id: 0,
        }
    }

    pub fn with_system_prompt(prompt: String) -> Self {
        let mut state = Self::new();
        state.system_prompt = Some(prompt);
        state
    }

    fn next_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Create a User fragment from the current input, clear input, and return
    /// the full message list suitable for sending to an LLM.
    pub fn send_message(&mut self) -> Vec<LLMMessage> {
        if self.input.is_empty() {
            return self.to_llm_messages();
        }
        let id = self.next_id();
        self.fragments.push(ChatFragment {
            role: ChatRole::User,
            content: self.input.clone(),
            id,
            blocks: None,
        });
        self.input.clear();
        self.input_cursor = 0;
        self.to_llm_messages()
    }

    /// Append an Assistant fragment with the given content
    pub fn receive_response(&mut self, content: String) {
        let id = self.next_id();
        self.fragments.push(ChatFragment {
            role: ChatRole::Assistant,
            content,
            id,
            blocks: None,
        });
    }

    /// Append a plain User fragment (no blocks). Returns its id.
    pub fn push_user(&mut self, content: String) -> usize {
        let id = self.next_id();
        self.fragments.push(ChatFragment {
            role: ChatRole::User,
            content,
            id,
            blocks: None,
        });
        id
    }

    /// Append a fragment with explicit role/content/blocks (used to restore a
    /// saved session verbatim). Returns its id.
    pub fn push_raw_fragment(
        &mut self,
        role: ChatRole,
        content: String,
        blocks: Option<Vec<ContentBlock>>,
    ) -> usize {
        let id = self.next_id();
        self.fragments.push(ChatFragment {
            role,
            content,
            id,
            blocks,
        });
        id
    }

    /// Append a fragment carrying structured agent blocks. `content` is a
    /// plain-text fallback (the concatenated text blocks) for display/legacy.
    pub fn push_agent_fragment(
        &mut self,
        role: ChatRole,
        content: String,
        blocks: Vec<ContentBlock>,
    ) -> usize {
        let id = self.next_id();
        self.fragments.push(ChatFragment {
            role,
            content,
            id,
            blocks: Some(blocks),
        });
        id
    }

    /// Build the conversation as provider-neutral [`AgentMessage`]s for the
    /// tool-calling loop. A `User` fragment carrying `tool_result` blocks maps
    /// to [`AgentMessage::ToolResults`] (Anthropic places results in a user
    /// turn); assistant fragments map to [`AgentMessage::Assistant`] preserving
    /// their `tool_use` blocks so multi-turn history is well-formed.
    pub fn to_agent_messages(&self) -> Vec<AgentMessage> {
        let mut messages = Vec::new();

        if let Some(ref prompt) = self.system_prompt {
            messages.push(AgentMessage::System(prompt.clone()));
        }
        if !self.injected_context.is_empty() {
            let context_str: Vec<String> = self
                .injected_context
                .iter()
                .map(|note| format!("# {}\n{}", note.title, note.content))
                .collect();
            messages.push(AgentMessage::System(format!(
                "Reference context:\n\n{}",
                context_str.join("\n\n---\n\n")
            )));
        }

        for frag in &self.fragments {
            match (&frag.role, &frag.blocks) {
                (ChatRole::System, _) => messages.push(AgentMessage::System(frag.content.clone())),
                (ChatRole::User, Some(blocks))
                    if blocks
                        .iter()
                        .any(|b| matches!(b, ContentBlock::ToolResult { .. })) =>
                {
                    messages.push(AgentMessage::ToolResults(blocks.clone()));
                }
                (ChatRole::User, _) => messages.push(AgentMessage::User(frag.content.clone())),
                (ChatRole::Assistant, Some(blocks)) => {
                    messages.push(AgentMessage::Assistant(blocks.clone()))
                }
                (ChatRole::Assistant, None) => {
                    messages.push(AgentMessage::Assistant(vec![ContentBlock::Text {
                        text: frag.content.clone(),
                    }]))
                }
            }
        }

        sanitize_tool_pairs(messages)
    }

    /// Build the full message list for sending to an LLM
    pub fn to_llm_messages(&self) -> Vec<LLMMessage> {
        let mut messages = Vec::new();

        // System prompt first
        if let Some(ref prompt) = self.system_prompt {
            messages.push(LLMMessage {
                role: ChatRole::System,
                content: prompt.clone(),
            });
        }

        // Injected context as a system message
        if !self.injected_context.is_empty() {
            let context_str: Vec<String> = self
                .injected_context
                .iter()
                .map(|note| format!("# {}\n{}", note.title, note.content))
                .collect();
            messages.push(LLMMessage {
                role: ChatRole::System,
                content: format!("Reference context:\n\n{}", context_str.join("\n\n---\n\n")),
            });
        }

        // Fragments in order
        for frag in &self.fragments {
            messages.push(LLMMessage {
                role: frag.role,
                content: frag.content.clone(),
            });
        }

        messages
    }

    /// Replace all fragments with the given role/content pairs
    pub fn load_fragments(&mut self, fragments: Vec<(ChatRole, String)>) {
        self.fragments = fragments
            .into_iter()
            .map(|(role, content)| {
                let id = self.next_id();
                ChatFragment {
                    role,
                    content,
                    id,
                    blocks: None,
                }
            })
            .collect();
    }

    // --- Context management ---

    pub fn inject_context(&mut self, notes: Vec<InjectedNote>) {
        self.injected_context.extend(notes);
    }

    pub fn clear_context(&mut self) {
        self.injected_context.clear();
    }

    pub fn remove_context(&mut self, idx: usize) {
        if idx < self.injected_context.len() {
            self.injected_context.remove(idx);
        }
    }

    // --- Selection ---

    pub fn toggle_select(&mut self, id: usize) {
        if !self.selected.remove(&id) {
            self.selected.insert(id);
        }
    }

    pub fn select_all(&mut self) {
        for frag in &self.fragments {
            self.selected.insert(frag.id);
        }
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    pub fn selected_fragments(&self) -> Vec<&ChatFragment> {
        self.fragments
            .iter()
            .filter(|f| self.selected.contains(&f.id))
            .collect()
    }

    // --- Edit mode ---

    /// Enter edit mode: collapse all fragments into a text buffer
    pub fn enter_edit_mode(&mut self) {
        self.edit_buffer = format_edit_buffer(&self.fragments);
        self.edit_cursor = (0, 0);
        self.edit_mode = true;
    }

    /// Exit edit mode: re-parse buffer back into fragments
    pub fn exit_edit_mode(&mut self) {
        let parsed = parse_edit_buffer(&self.edit_buffer);
        self.fragments = parsed
            .into_iter()
            .map(|(role, content)| {
                let id = self.next_id();
                ChatFragment {
                    role,
                    content,
                    id,
                    blocks: None,
                }
            })
            .collect();
        self.edit_mode = false;
    }

    // --- Input editing ---

    pub fn insert_input_char(&mut self, c: char) {
        let pos = self.input_cursor.min(self.input.len());
        self.input.insert(pos, c);
        self.input_cursor = pos + 1;
    }

    pub fn delete_input_char(&mut self) {
        if self.input_cursor > 0 {
            let pos = self.input_cursor.min(self.input.len());
            if pos > 0 {
                self.input.remove(pos - 1);
                self.input_cursor = pos - 1;
            }
        }
    }

    pub fn input_cursor_left(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor -= 1;
        }
    }

    pub fn input_cursor_right(&mut self) {
        if self.input_cursor < self.input.len() {
            self.input_cursor += 1;
        }
    }
}

/// Format fragments into an editable text buffer with `[role]` headers
/// separated by `\n---\n`.
pub fn format_edit_buffer(fragments: &[ChatFragment]) -> String {
    fragments
        .iter()
        .map(|f| format!("[{}]\n{}", f.role.as_str(), f.content))
        .collect::<Vec<_>>()
        .join("\n---\n")
}

/// Parse an edit buffer back into (role, content) pairs.
///
/// Splits on `\n---\n`, detects `[role]` headers on first line of each chunk.
/// If no header is found, infers role by alternating user/assistant pattern.
pub fn parse_edit_buffer(buffer: &str) -> Vec<(ChatRole, String)> {
    if buffer.is_empty() {
        return Vec::new();
    }

    let chunks: Vec<&str> = buffer.split("\n---\n").collect();
    let mut results = Vec::new();
    let mut last_role = ChatRole::Assistant; // so first inferred role is User

    for chunk in chunks {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }

        // Try to detect [role] header on first line
        let mut lines = chunk.lines();
        if let Some(first_line) = lines.next() {
            let trimmed = first_line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                let role_str = &trimmed[1..trimmed.len() - 1];
                if let Some(role) = ChatRole::from_str(role_str) {
                    let content: String = lines.collect::<Vec<_>>().join("\n");
                    results.push((role, content));
                    last_role = role;
                    continue;
                }
            }
        }

        // No valid header — infer alternating role
        let inferred = match last_role {
            ChatRole::User | ChatRole::System => ChatRole::Assistant,
            ChatRole::Assistant => ChatRole::User,
        };
        results.push((inferred, chunk.to_string()));
        last_role = inferred;
    }

    results
}

/// Repair a conversation so every `tool_use` is answered by a matching
/// `tool_result` and no `tool_result` dangles. The Anthropic API 400s if an
/// assistant `tool_use` block is not immediately followed by a user message
/// carrying a `tool_result` for each id (and vice-versa). Editing the chat —
/// deleting a tool call or its result — easily produces such an imbalance, so
/// we normalize here, at the single point where history is serialized for the
/// model, rather than trusting every editor path to keep pairs intact.
///
/// Rules applied, in order:
/// - An assistant turn left empty by editing is dropped.
/// - For an assistant turn with `tool_use` blocks, the immediately-following
///   `ToolResults` (if any) is consumed; results that match one of this turn's
///   ids are kept, others discarded.
/// - Any `tool_use` left unanswered gets a synthetic `is_error` result so the
///   model sees the call resolved (it can recover) rather than the request 400ing.
/// - A `ToolResults` message with no preceding `tool_use` to answer is dropped.
fn sanitize_tool_pairs(messages: Vec<AgentMessage>) -> Vec<AgentMessage> {
    let mut out: Vec<AgentMessage> = Vec::with_capacity(messages.len());
    let mut i = 0;
    while i < messages.len() {
        match &messages[i] {
            AgentMessage::System(s) => {
                out.push(AgentMessage::System(s.clone()));
                i += 1;
            }
            AgentMessage::User(s) => {
                out.push(AgentMessage::User(s.clone()));
                i += 1;
            }
            // Orphan results (the assistant turn they answered was edited away).
            AgentMessage::ToolResults(_) => {
                i += 1;
            }
            AgentMessage::Assistant(blocks) => {
                if blocks.is_empty() {
                    i += 1;
                    continue;
                }
                let use_ids: Vec<String> = blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolUse { id, .. } => Some(id.clone()),
                        _ => None,
                    })
                    .collect();
                out.push(AgentMessage::Assistant(blocks.clone()));
                if use_ids.is_empty() {
                    i += 1;
                    continue;
                }

                let mut results: Vec<ContentBlock> = Vec::new();
                let mut answered: HashSet<String> = HashSet::new();
                if let Some(AgentMessage::ToolResults(rblocks)) = messages.get(i + 1) {
                    for b in rblocks {
                        if let ContentBlock::ToolResult { tool_use_id, .. } = b {
                            if use_ids.contains(tool_use_id) && answered.insert(tool_use_id.clone())
                            {
                                results.push(b.clone());
                            }
                        }
                    }
                    i += 2;
                } else {
                    i += 1;
                }
                for id in &use_ids {
                    if !answered.contains(id) {
                        results.push(ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content: "[tool result unavailable — removed when the conversation was edited]"
                                .to_string(),
                            is_error: true,
                        });
                    }
                }
                out.push(AgentMessage::ToolResults(results));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_use(id: &str) -> ContentBlock {
        ContentBlock::ToolUse {
            id: id.into(),
            name: "search_events".into(),
            input: serde_json::json!({}),
        }
    }
    fn tool_result(id: &str) -> ContentBlock {
        ContentBlock::ToolResult {
            tool_use_id: id.into(),
            content: "ok".into(),
            is_error: false,
        }
    }

    #[test]
    fn sanitize_synthesizes_missing_tool_result() {
        // Assistant calls a tool but its result was deleted — must not dangle.
        let msgs = vec![
            AgentMessage::User("hi".into()),
            AgentMessage::Assistant(vec![tool_use("a")]),
        ];
        let out = sanitize_tool_pairs(msgs);
        assert_eq!(out.len(), 3);
        match &out[2] {
            AgentMessage::ToolResults(blocks) => match &blocks[0] {
                ContentBlock::ToolResult {
                    tool_use_id,
                    is_error,
                    ..
                } => {
                    assert_eq!(tool_use_id, "a");
                    assert!(is_error);
                }
                _ => panic!("expected synthetic tool_result"),
            },
            _ => panic!("expected ToolResults after assistant tool_use"),
        }
    }

    #[test]
    fn sanitize_drops_orphan_tool_result() {
        // A tool_result whose assistant turn was deleted is dropped.
        let msgs = vec![
            AgentMessage::ToolResults(vec![tool_result("gone")]),
            AgentMessage::User("hi".into()),
        ];
        let out = sanitize_tool_pairs(msgs);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], AgentMessage::User(_)));
    }

    #[test]
    fn sanitize_keeps_well_formed_pairs() {
        let msgs = vec![
            AgentMessage::User("hi".into()),
            AgentMessage::Assistant(vec![tool_use("a")]),
            AgentMessage::ToolResults(vec![tool_result("a")]),
            AgentMessage::Assistant(vec![ContentBlock::Text {
                text: "done".into(),
            }]),
        ];
        let out = sanitize_tool_pairs(msgs);
        assert_eq!(out.len(), 4);
        match &out[2] {
            AgentMessage::ToolResults(blocks) => assert_eq!(blocks.len(), 1),
            _ => panic!("expected preserved ToolResults"),
        }
    }

    #[test]
    fn test_chat_state_new_defaults() {
        let state = ChatState::new();
        assert!(state.fragments.is_empty());
        assert!(state.input.is_empty());
        assert!(!state.edit_mode);
        assert!(!state.generating);
        assert!(state.system_prompt.is_none());
        assert!(state.injected_context.is_empty());
    }

    #[test]
    fn test_chat_send_message_creates_fragment_and_clears_input() {
        let mut state = ChatState::new();
        state.input = "Hello LLM".into();
        state.input_cursor = 9;

        let messages = state.send_message();
        assert_eq!(state.fragments.len(), 1);
        assert_eq!(state.fragments[0].role, ChatRole::User);
        assert_eq!(state.fragments[0].content, "Hello LLM");
        assert!(state.input.is_empty());
        assert_eq!(state.input_cursor, 0);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Hello LLM");
    }

    #[test]
    fn test_chat_receive_response() {
        let mut state = ChatState::new();
        state.receive_response("Hello human".into());
        assert_eq!(state.fragments.len(), 1);
        assert_eq!(state.fragments[0].role, ChatRole::Assistant);
        assert_eq!(state.fragments[0].content, "Hello human");
    }

    #[test]
    fn test_chat_send_receive_roundtrip_order() {
        let mut state = ChatState::new();
        state.input = "question".into();
        state.send_message();
        state.receive_response("answer".into());
        state.input = "follow-up".into();
        state.send_message();

        assert_eq!(state.fragments.len(), 3);
        assert_eq!(state.fragments[0].role, ChatRole::User);
        assert_eq!(state.fragments[1].role, ChatRole::Assistant);
        assert_eq!(state.fragments[2].role, ChatRole::User);
    }

    #[test]
    fn test_chat_to_llm_messages_with_system_prompt() {
        let mut state = ChatState::with_system_prompt("You are helpful.".into());
        state.input = "Hi".into();
        let messages = state.send_message();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, ChatRole::System);
        assert_eq!(messages[0].content, "You are helpful.");
        assert_eq!(messages[1].role, ChatRole::User);
    }

    #[test]
    fn test_chat_to_llm_messages_with_context() {
        let mut state = ChatState::new();
        state.inject_context(vec![InjectedNote {
            addr: None,
            title: "Note 1".into(),
            content: "Context content".into(),
        }]);
        state.input = "question".into();
        let messages = state.send_message();

        // Should have context system message + user message
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, ChatRole::System);
        assert!(messages[0].content.contains("Context content"));
        assert_eq!(messages[1].role, ChatRole::User);
    }

    #[test]
    fn test_chat_to_llm_messages_without_context() {
        let mut state = ChatState::new();
        state.input = "Hi".into();
        let messages = state.send_message();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, ChatRole::User);
    }

    #[test]
    fn test_chat_inject_clear_remove_context() {
        let mut state = ChatState::new();
        state.inject_context(vec![
            InjectedNote {
                addr: None,
                title: "A".into(),
                content: "aaa".into(),
            },
            InjectedNote {
                addr: None,
                title: "B".into(),
                content: "bbb".into(),
            },
        ]);
        assert_eq!(state.injected_context.len(), 2);

        state.remove_context(0);
        assert_eq!(state.injected_context.len(), 1);
        assert_eq!(state.injected_context[0].title, "B");

        state.clear_context();
        assert!(state.injected_context.is_empty());
    }

    #[test]
    fn test_chat_toggle_select() {
        let mut state = ChatState::new();
        state.input = "a".into();
        state.send_message();
        let id = state.fragments[0].id;

        state.toggle_select(id);
        assert!(state.selected.contains(&id));
        state.toggle_select(id);
        assert!(!state.selected.contains(&id));
    }

    #[test]
    fn test_chat_select_all_and_clear() {
        let mut state = ChatState::new();
        state.input = "a".into();
        state.send_message();
        state.receive_response("b".into());

        state.select_all();
        assert_eq!(state.selected.len(), 2);
        assert_eq!(state.selected_fragments().len(), 2);

        state.clear_selection();
        assert!(state.selected.is_empty());
    }

    #[test]
    fn test_chat_selected_fragments_ordering() {
        let mut state = ChatState::new();
        state.input = "first".into();
        state.send_message();
        state.receive_response("second".into());
        state.input = "third".into();
        state.send_message();

        state.select_all();
        let selected = state.selected_fragments();
        assert_eq!(selected[0].content, "first");
        assert_eq!(selected[1].content, "second");
        assert_eq!(selected[2].content, "third");
    }

    #[test]
    fn test_chat_enter_edit_mode_format() {
        let mut state = ChatState::new();
        state.input = "hello".into();
        state.send_message();
        state.receive_response("world".into());

        state.enter_edit_mode();
        assert!(state.edit_mode);
        assert!(state.edit_buffer.contains("[user]"));
        assert!(state.edit_buffer.contains("[assistant]"));
        assert!(state.edit_buffer.contains("hello"));
        assert!(state.edit_buffer.contains("world"));
    }

    #[test]
    fn test_chat_exit_edit_mode_roundtrip() {
        let mut state = ChatState::new();
        state.input = "hello".into();
        state.send_message();
        state.receive_response("world".into());

        state.enter_edit_mode();
        state.exit_edit_mode();

        assert!(!state.edit_mode);
        assert_eq!(state.fragments.len(), 2);
        assert_eq!(state.fragments[0].role, ChatRole::User);
        assert_eq!(state.fragments[0].content, "hello");
        assert_eq!(state.fragments[1].role, ChatRole::Assistant);
        assert_eq!(state.fragments[1].content, "world");
    }

    #[test]
    fn test_parse_edit_buffer_with_role_headers() {
        let buffer = "[user]\nHello\n---\n[assistant]\nHi there";
        let parsed = parse_edit_buffer(buffer);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, ChatRole::User);
        assert_eq!(parsed[0].1, "Hello");
        assert_eq!(parsed[1].0, ChatRole::Assistant);
        assert_eq!(parsed[1].1, "Hi there");
    }

    #[test]
    fn test_parse_edit_buffer_without_headers() {
        let buffer = "Hello\n---\nHi there";
        let parsed = parse_edit_buffer(buffer);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, ChatRole::User);
        assert_eq!(parsed[1].0, ChatRole::Assistant);
    }

    #[test]
    fn test_parse_edit_buffer_merge() {
        // Single chunk (no ---) → single fragment
        let buffer = "[user]\nAll one message";
        let parsed = parse_edit_buffer(buffer);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].1, "All one message");
    }

    #[test]
    fn test_parse_edit_buffer_split() {
        // Adding --- creates new fragment
        let buffer = "[user]\nPart 1\n---\n[user]\nPart 2";
        let parsed = parse_edit_buffer(buffer);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].1, "Part 1");
        assert_eq!(parsed[1].1, "Part 2");
    }

    #[test]
    fn test_chat_input_char_insertion() {
        let mut state = ChatState::new();
        state.insert_input_char('H');
        state.insert_input_char('i');
        assert_eq!(state.input, "Hi");
        assert_eq!(state.input_cursor, 2);
    }

    #[test]
    fn test_chat_input_backspace() {
        let mut state = ChatState::new();
        state.insert_input_char('A');
        state.insert_input_char('B');
        state.delete_input_char();
        assert_eq!(state.input, "A");
        assert_eq!(state.input_cursor, 1);
    }

    #[test]
    fn test_chat_input_cursor_bounds() {
        let mut state = ChatState::new();
        // Left at 0 should stay at 0
        state.input_cursor_left();
        assert_eq!(state.input_cursor, 0);

        state.insert_input_char('X');
        // Right past end should stay at end
        state.input_cursor_right();
        assert_eq!(state.input_cursor, 1);

        state.input_cursor_left();
        assert_eq!(state.input_cursor, 0);
    }
}
