//! Telemetry module — all telemetry is permanently disabled in Articulate (a8e).
//! This module provides no-op stubs so dependent code compiles without changes.

pub const TELEMETRY_ENABLED_KEY: &str = "A8E_TELEMETRY_ENABLED";

pub fn get_telemetry_choice() -> Option<bool> {
    Some(false)
}

pub fn is_telemetry_enabled() -> bool {
    false
}

pub fn set_session_context(_interface: &str, _is_resumed: bool) {}

pub fn emit_session_started() {}

#[derive(Default, Clone)]
pub struct ErrorContext {
    pub component: Option<String>,
    pub action: Option<String>,
    pub error_message: Option<String>,
}

pub fn emit_error(_error_type: &str, _error_message: &str) {}

pub fn emit_error_with_context(_error_type: &str, _context: ErrorContext) {}

pub fn emit_custom_slash_command_used() {}

pub fn classify_error(_error: &str) -> &'static str {
    "unknown_error"
}

pub async fn emit_event(
    _event_name: &str,
    _properties: std::collections::HashMap<String, serde_json::Value>,
) -> Result<(), String> {
    Ok(())
}
