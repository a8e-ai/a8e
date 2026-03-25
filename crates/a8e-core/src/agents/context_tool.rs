//! Context management tool handlers for the Articulate agent
//!
//! Exposes compact, clear, and status actions as a platform tool so the agent
//! can programmatically manage its own conversation context. This is especially
//! useful for scheduled/loop tasks that need clean context between iterations.

use crate::context_mgmt::{
    check_if_compaction_needed, compact_messages, DEFAULT_COMPACTION_THRESHOLD,
};
use crate::conversation::Conversation;
use crate::mcp_utils::ToolResult;
use crate::session::Session;
use crate::token_counter::create_token_counter;
use rmcp::model::{Content, ErrorCode, ErrorData};
use tracing::info;

use super::Agent;

impl Agent {
    /// Handle context management tool calls
    pub async fn handle_context_management(
        &self,
        arguments: serde_json::Value,
        session: &Session,
    ) -> ToolResult<Vec<Content>> {
        let action = arguments
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "Missing 'action' parameter".to_string(),
                    None,
                )
            })?;

        let reason = arguments
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("agent requested");

        match action {
            "compact" => self.handle_context_compact(session, reason).await,
            "clear" => self.handle_context_clear(session, reason).await,
            "status" => self.handle_context_status(session).await,
            _ => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Unknown context action: {}", action),
                None,
            )),
        }
    }

    async fn handle_context_compact(
        &self,
        session: &Session,
        reason: &str,
    ) -> ToolResult<Vec<Content>> {
        info!(
            session_id = %session.id,
            reason = reason,
            "Agent-initiated context compaction"
        );

        let manager = self.config.session_manager.clone();
        let full_session = manager.get_session(&session.id, true).await.map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to load session: {}", e),
                None,
            )
        })?;

        let conversation = full_session.conversation.ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "Session has no conversation".to_string(),
                None,
            )
        })?;

        let message_count = conversation.messages().len();

        let provider = self.provider().await.map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to get provider: {}", e),
                None,
            )
        })?;

        let (compacted_conversation, usage) =
            compact_messages(provider.as_ref(), &session.id, &conversation, true)
                .await
                .map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Compaction failed: {}", e),
                        None,
                    )
                })?;

        let compacted_count = compacted_conversation.messages().len();

        manager
            .replace_conversation(&session.id, &compacted_conversation)
            .await
            .map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to replace conversation: {}", e),
                    None,
                )
            })?;

        self.update_session_metrics(&session.id, full_session.schedule_id, &usage, true)
            .await
            .map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to update metrics: {}", e),
                    None,
                )
            })?;

        Ok(vec![Content::text(format!(
            "Context compacted successfully.\n\
             Messages before: {}\n\
             Messages after: {}\n\
             Reason: {}",
            message_count, compacted_count, reason
        ))])
    }

    async fn handle_context_clear(
        &self,
        session: &Session,
        reason: &str,
    ) -> ToolResult<Vec<Content>> {
        info!(
            session_id = %session.id,
            reason = reason,
            "Agent-initiated context clear"
        );

        let manager = self.config.session_manager.clone();

        manager
            .replace_conversation(&session.id, &Conversation::default())
            .await
            .map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to clear conversation: {}", e),
                    None,
                )
            })?;

        manager
            .update(&session.id)
            .total_tokens(Some(0))
            .input_tokens(Some(0))
            .output_tokens(Some(0))
            .apply()
            .await
            .map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to reset token counters: {}", e),
                    None,
                )
            })?;

        Ok(vec![Content::text(format!(
            "Context cleared successfully. Conversation history and token counters have been reset.\n\
             Reason: {}",
            reason
        ))])
    }

    async fn handle_context_status(&self, session: &Session) -> ToolResult<Vec<Content>> {
        let manager = self.config.session_manager.clone();
        let full_session = manager.get_session(&session.id, true).await.map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to load session: {}", e),
                None,
            )
        })?;

        let conversation = full_session.conversation.as_ref().ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "Session has no conversation".to_string(),
                None,
            )
        })?;

        let message_count = conversation.messages().len();
        let agent_visible_count = conversation
            .messages()
            .iter()
            .filter(|m| m.is_agent_visible())
            .count();

        let provider = self.provider().await.map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to get provider: {}", e),
                None,
            )
        })?;

        let context_limit = provider.get_model_config().context_limit();

        let current_tokens = match full_session.total_tokens {
            Some(tokens) => tokens as usize,
            None => {
                let counter = create_token_counter().await.map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to create token counter: {}", e),
                        None,
                    )
                })?;
                conversation
                    .messages()
                    .iter()
                    .filter(|m| m.is_agent_visible())
                    .map(|msg| counter.count_chat_tokens("", std::slice::from_ref(msg), &[]))
                    .sum()
            }
        };

        let usage_ratio = if context_limit > 0 {
            current_tokens as f64 / context_limit as f64
        } else {
            0.0
        };

        let needs_compaction =
            check_if_compaction_needed(provider.as_ref(), conversation, None, &full_session)
                .await
                .unwrap_or(false);

        let threshold = crate::config::Config::global()
            .get_param::<f64>("A8E_AUTO_COMPACT_THRESHOLD")
            .unwrap_or(DEFAULT_COMPACTION_THRESHOLD);

        Ok(vec![Content::text(format!(
            "Context Status:\n\
             Total messages: {}\n\
             Agent-visible messages: {}\n\
             Estimated tokens: {}\n\
             Context limit: {}\n\
             Usage: {:.1}%\n\
             Auto-compact threshold: {:.0}%\n\
             Needs compaction: {}",
            message_count,
            agent_visible_count,
            current_tokens,
            context_limit,
            usage_ratio * 100.0,
            threshold * 100.0,
            needs_compaction
        ))])
    }
}
