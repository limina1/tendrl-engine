//! Claude Code session reader
//!
//! Parses Claude Code conversation JSONL files from ~/.claude/projects/

use crate::error::{EngineError, Result};
use serde::Serialize;
use std::io::BufRead;
use std::path::{Path, PathBuf};

/// Generate a v4-style UUID from random bytes
fn gen_uuid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    // Simple pseudo-random using time + pid
    let pid = std::process::id() as u128;
    let r = seed.wrapping_mul(6364136223846793005).wrapping_add(pid);
    let bytes: [u8; 16] = r.to_le_bytes();
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-4{:01x}{:02x}-{:01x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6] & 0x0f, bytes[7],
        0x8 | (bytes[8] & 0x03), bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

/// ISO 8601 timestamp in UTC
fn iso_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs();
    let millis = dur.subsec_millis();
    // Convert to date/time components
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;
    // Days since epoch to y/m/d (simplified algorithm)
    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let year_days = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
        if remaining < year_days { break; }
        remaining -= year_days;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md as i64 { m = i + 1; break; }
        remaining -= md as i64;
    }
    let d = remaining + 1;
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", y, m, d, hours, minutes, seconds, millis)
}

#[derive(Debug, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub date: String,
    pub message_count: usize,
    pub first_prompt: String,
    pub last_message: String,
    pub modified: u64,
}

/// Block type within a message
#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type")]
pub enum SessionBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    #[serde(rename = "tool_use")]
    ToolUse { name: String, input: serde_json::Value },
    #[serde(rename = "tool_result")]
    ToolResult { content: String },
}

#[derive(Debug, Serialize)]
pub struct SessionMessage {
    pub role: String,
    pub blocks: Vec<SessionBlock>,
    pub timestamp: String,
}

#[derive(Debug, Serialize)]
pub struct SessionDetail {
    pub id: String,
    pub messages: Vec<SessionMessage>,
}

/// Encode a project path to Claude's directory format (/ becomes -)
pub fn resolve_claude_dir(project_path: &Path) -> PathBuf {
    let encoded = project_path
        .to_string_lossy()
        .replace('/', "-");
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join(".claude")
        .join("projects")
        .join(encoded)
}

/// Extract text content from a message's content field
fn extract_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => {
            arr.iter()
                .filter_map(|block| {
                    if block.get("type")?.as_str()? == "text" {
                        block.get("text")?.as_str().map(String::from)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
        _ => String::new(),
    }
}

/// List all sessions in a Claude project directory
pub fn list_sessions(claude_dir: &Path) -> Result<Vec<SessionSummary>> {
    let mut sessions = Vec::new();

    let mut entries: Vec<_> = std::fs::read_dir(claude_dir)
        .map_err(|e| EngineError::Database(format!("Failed to read sessions dir: {e}")))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map_or(false, |ext| ext == "jsonl")
        })
        .collect();

    // Sort by mtime descending (most recent first)
    entries.sort_by(|a, b| {
        let ma = a.metadata().and_then(|m| m.modified()).ok();
        let mb = b.metadata().and_then(|m| m.modified()).ok();
        mb.cmp(&ma)
    });

    for entry in entries {
        let path = entry.path();
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };

        let mut date = String::new();
        let mut message_count = 0usize;
        let mut first_prompt = String::new();
        let mut last_message = String::new();

        for line in std::io::BufReader::new(file).lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            if line.trim().is_empty() {
                continue;
            }
            let msg: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if msg_type != "user" && msg_type != "assistant" {
                continue;
            }

            message_count += 1;

            if date.is_empty() {
                if let Some(ts) = msg.get("timestamp").and_then(|v| v.as_str()) {
                    date = ts.chars().take(10).collect();
                }
            }

            if let Some(content) = msg.get("message").and_then(|m| m.get("content")) {
                let text = extract_text(content);
                if !text.is_empty() && !text.starts_with("[Request interrupted") {
                    if first_prompt.is_empty() && msg_type == "user" {
                        first_prompt = text.chars().take(200).collect();
                    }
                    last_message = text.chars().take(200).collect();
                }
            }
        }

        if message_count == 0 {
            continue;
        }

        sessions.push(SessionSummary {
            id,
            date,
            message_count,
            first_prompt,
            last_message,
            modified,
        });
    }

    Ok(sessions)
}

/// Get full conversation messages for a session (by ID or prefix)
pub fn get_session(claude_dir: &Path, session_id: &str) -> Result<SessionDetail> {
    // Find matching file (supports prefix matching)
    let matches: Vec<_> = std::fs::read_dir(claude_dir)
        .map_err(|e| EngineError::Database(format!("Failed to read sessions dir: {e}")))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with(session_id) && name.ends_with(".jsonl")
        })
        .map(|e| e.path())
        .collect();

    if matches.is_empty() {
        return Err(EngineError::NotFound(format!(
            "No session matching '{session_id}'"
        )));
    }
    if matches.len() > 1 {
        return Err(EngineError::InvalidFilter(format!(
            "Multiple sessions match '{}': {}",
            session_id,
            matches.len()
        )));
    }

    let path = &matches[0];
    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    let file = std::fs::File::open(path)
        .map_err(|e| EngineError::Database(format!("Failed to open session: {e}")))?;

    let messages = parse_session_messages(path, 0)?;
    Ok(SessionDetail { id, messages })
}

/// Parse messages from a session file, optionally skipping the first `offset` messages.
pub fn parse_session_messages(path: &Path, offset: usize) -> Result<Vec<SessionMessage>> {
    let file = std::fs::File::open(path)
        .map_err(|e| EngineError::Database(format!("Failed to open session: {e}")))?;

    let mut messages = Vec::new();
    let mut index = 0usize;

    for line in std::io::BufReader::new(file).lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        let msg: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if msg_type != "user" && msg_type != "assistant" {
            continue;
        }

        let message = match msg.get("message") {
            Some(m) => m,
            None => continue,
        };

        let role = message
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or(msg_type)
            .to_string();

        let blocks = extract_blocks(message.get("content"));

        // Skip messages with no meaningful content
        if blocks.is_empty() {
            continue;
        }
        // Skip "[Request interrupted" messages
        if blocks.len() == 1 {
            if let SessionBlock::Text { text } = &blocks[0] {
                if text.starts_with("[Request interrupted") {
                    continue;
                }
            }
        }

        index += 1;
        if index <= offset {
            continue;
        }

        let timestamp = msg
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        messages.push(SessionMessage {
            role,
            blocks,
            timestamp,
        });
    }

    Ok(messages)
}

/// Append a user message to a session JSONL file.
/// Returns the uuid of the appended message.
pub fn append_message(claude_dir: &Path, session_id: &str, content: &str) -> Result<String> {
    // Find matching file
    let matches: Vec<_> = std::fs::read_dir(claude_dir)
        .map_err(|e| EngineError::Database(format!("Failed to read sessions dir: {e}")))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with(session_id) && name.ends_with(".jsonl")
        })
        .map(|e| e.path())
        .collect();

    if matches.is_empty() {
        return Err(EngineError::NotFound(format!("No session matching '{session_id}'")));
    }

    let path = &matches[0];
    let full_session_id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();

    // Find the last message's uuid to use as parentUuid
    let file_content = std::fs::read_to_string(path)
        .map_err(|e| EngineError::Database(format!("Failed to read session: {e}")))?;

    let mut last_uuid: Option<String> = None;
    let mut cwd = String::new();
    let mut git_branch = String::new();

    for line in file_content.lines().rev() {
        if line.trim().is_empty() { continue; }
        if let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) {
            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if (msg_type == "user" || msg_type == "assistant") && last_uuid.is_none() {
                last_uuid = msg.get("uuid").and_then(|v| v.as_str()).map(String::from);
            }
            if cwd.is_empty() {
                if let Some(c) = msg.get("cwd").and_then(|v| v.as_str()) {
                    cwd = c.to_string();
                }
            }
            if git_branch.is_empty() {
                if let Some(b) = msg.get("gitBranch").and_then(|v| v.as_str()) {
                    git_branch = b.to_string();
                }
            }
            if last_uuid.is_some() && !cwd.is_empty() && !git_branch.is_empty() {
                break;
            }
        }
    }

    let new_uuid = gen_uuid();
    let timestamp = iso_now();

    let entry = serde_json::json!({
        "parentUuid": last_uuid,
        "isSidechain": false,
        "type": "user",
        "message": {
            "role": "user",
            "content": content
        },
        "uuid": new_uuid,
        "timestamp": timestamp,
        "userType": "external",
        "cwd": if cwd.is_empty() { std::env::current_dir().unwrap_or_default().to_string_lossy().to_string() } else { cwd },
        "sessionId": full_session_id,
        "gitBranch": if git_branch.is_empty() { "main".to_string() } else { git_branch },
    });

    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|e| EngineError::Database(format!("Failed to open session for append: {e}")))?;
    writeln!(file, "{}", serde_json::to_string(&entry).unwrap())
        .map_err(|e| EngineError::Database(format!("Failed to write to session: {e}")))?;

    Ok(new_uuid)
}

/// Extract structured blocks from a message content field
fn extract_blocks(content: Option<&serde_json::Value>) -> Vec<SessionBlock> {
    let content = match content {
        Some(c) => c,
        None => return vec![],
    };

    match content {
        serde_json::Value::String(s) => {
            if s.trim().is_empty() {
                vec![]
            } else {
                vec![SessionBlock::Text { text: s.clone() }]
            }
        }
        serde_json::Value::Array(arr) => {
            let mut blocks = Vec::new();
            for item in arr {
                let block_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match block_type {
                    "text" => {
                        let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        if !text.is_empty() {
                            blocks.push(SessionBlock::Text { text: text.to_string() });
                        }
                    }
                    "thinking" => {
                        let thinking = item.get("thinking").and_then(|v| v.as_str()).unwrap_or("");
                        if !thinking.is_empty() {
                            blocks.push(SessionBlock::Thinking { thinking: thinking.to_string() });
                        }
                    }
                    "tool_use" => {
                        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let input = item.get("input").cloned().unwrap_or(serde_json::Value::Null);
                        blocks.push(SessionBlock::ToolUse { name, input });
                    }
                    "tool_result" => {
                        let content_val = item.get("content");
                        let text = match content_val {
                            Some(serde_json::Value::String(s)) => s.clone(),
                            Some(serde_json::Value::Array(arr)) => {
                                arr.iter()
                                    .filter_map(|b| {
                                        if b.get("type")?.as_str()? == "text" {
                                            b.get("text")?.as_str().map(String::from)
                                        } else {
                                            None
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            }
                            _ => String::new(),
                        };
                        if !text.is_empty() {
                            blocks.push(SessionBlock::ToolResult { content: text });
                        }
                    }
                    _ => {}
                }
            }
            blocks
        }
        _ => vec![],
    }
}
