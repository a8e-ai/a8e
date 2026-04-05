//! Session-Scoped Loop Scheduler (MCP Server)
//!
//! Pushes natural-language prompts into the main agent interaction on a
//! schedule, as if the user had typed them.  This gives loop tasks full
//! agent capabilities (tool use, reasoning, multi-step tasks) instead of
//! being limited to simple shell commands.
//!
//! Key behaviour:
//! - When a job fires and the agent is **idle**, the prompt is placed in
//!   a shared queue for the main session loop to consume.
//! - When the agent is **busy**, the execution is skipped.
//! - All jobs are session-scoped and cleaned up on process exit.

use indoc::formatdoc;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, Content, ErrorCode, ErrorData, Implementation, ServerCapabilities,
        ServerInfo,
    },
    schemars::JsonSchema,
    service::RequestContext,
    tool, tool_handler, tool_router, RoleServer, ServerHandler,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{collections::HashMap, sync::Arc, sync::OnceLock};
use tokio::sync::Mutex;

const MAX_JOBS_PER_SESSION: usize = 20;

// ---------------------------------------------------------------------------
// Global loop channel — bridges the MCP extension and the main session loop
// ---------------------------------------------------------------------------

static AGENT_BUSY: AtomicBool = AtomicBool::new(false);

struct GlobalLoopChannel {
    sender: tokio::sync::mpsc::UnboundedSender<LoopPromptEvent>,
    receiver: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<LoopPromptEvent>>>,
}

static LOOP_CHANNEL: OnceLock<GlobalLoopChannel> = OnceLock::new();

fn get_or_init_channel() -> &'static GlobalLoopChannel {
    LOOP_CHANNEL.get_or_init(|| {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        GlobalLoopChannel {
            sender: tx,
            receiver: std::sync::Mutex::new(Some(rx)),
        }
    })
}

/// Take the global loop prompt receiver.  Call once from the session loop.
/// Returns `None` if already taken.
pub fn take_loop_prompt_receiver() -> Option<tokio::sync::mpsc::UnboundedReceiver<LoopPromptEvent>>
{
    get_or_init_channel().receiver.lock().unwrap().take()
}

/// Set the global "agent is busy" flag.
pub fn set_agent_busy(busy: bool) {
    AGENT_BUSY.store(busy, Ordering::SeqCst);
}

fn is_agent_busy() -> bool {
    AGENT_BUSY.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopJobInfo {
    pub id: String,
    pub schedule: String,
    /// Natural-language prompt injected into the agent conversation
    pub prompt: String,
    pub cwd: Option<String>,
    /// If true, clear conversation context before each loop iteration
    #[serde(default)]
    pub clear: bool,
    pub created_at: String,
    pub last_run_at: Option<String>,
    pub next_run_at: Option<String>,
    pub run_count: u64,
    pub skip_count: u64,
    pub last_result: Option<LoopJobResult>,
    pub status: LoopJobStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopJobResult {
    pub success: bool,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LoopJobStatus {
    Active,
    Paused,
}

/// A prompt event that should be injected into the main agent conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopPromptEvent {
    pub job_id: String,
    pub prompt: String,
    pub cwd: Option<String>,
    pub schedule: String,
    /// If true, the consumer should clear conversation context before processing
    #[serde(default)]
    pub clear: bool,
}

// Legacy aliases for backward compatibility
pub type CronJobInfo = LoopJobInfo;
pub type CronJobResult = LoopJobResult;
pub type CronJobStatus = LoopJobStatus;
pub type CronPromptEvent = LoopPromptEvent;
pub type CronServer = LoopServer;
pub fn take_cron_prompt_receiver() -> Option<tokio::sync::mpsc::UnboundedReceiver<LoopPromptEvent>>
{
    take_loop_prompt_receiver()
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

struct InternalJob {
    info: LoopJobInfo,
    cancel_token: tokio_util::sync::CancellationToken,
}

type JobRegistry = Arc<Mutex<HashMap<String, InternalJob>>>;

// ---------------------------------------------------------------------------
// MCP Tool Params
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LoopCreateParams {
    /// Schedule expression. Supported formats:
    /// - "every 5m", "every 1h", "every 30s" (fixed interval)
    /// - "every hour at :15" (hourly at specific minute)
    /// - Cron expression: "*/10 * * * *" (every 10 min), "0 9 * * 1" (Monday 9AM)
    pub schedule: String,
    /// Natural-language prompt to inject into the agent conversation on each
    /// trigger. Equivalent to the user typing this message. The agent will
    /// process it with full tool access and reasoning capabilities.
    pub prompt: String,
    /// Working directory context for the prompt (optional)
    #[serde(default)]
    pub cwd: Option<String>,
    /// If true, clear conversation context before each loop iteration.
    /// Ensures each run starts with a fresh context, avoiding interference
    /// from accumulated history. Recommended for long-running or independent tasks.
    #[serde(default)]
    pub clear: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LoopListParams {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LoopRemoveParams {
    /// The ID of the loop job to remove
    pub job_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LoopGetParams {
    /// The ID of the loop job
    pub job_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LoopPauseParams {
    /// The ID of the loop job to pause
    pub job_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LoopResumeParams {
    /// The ID of the loop job to resume
    pub job_id: String,
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct LoopServer {
    tool_router: ToolRouter<Self>,
    instructions: String,
    jobs: JobRegistry,
}

impl Default for LoopServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router(router = tool_router)]
impl LoopServer {
    pub fn new() -> Self {
        let instructions = formatdoc! {r#"
            Session-scoped loop task management with prompt injection.
            Create, list, pause, resume, and remove recurring agent prompts
            that fire on a schedule within the current CLI session.
            Each job stores a natural-language prompt that is injected into
            the main agent conversation when triggered — equivalent to the
            user typing the message. If the agent is busy, the prompt is
            skipped. All jobs are automatically cleaned up when the session
            ends.
            IMPORTANT: Loop tasks are LOCAL and SESSION-SCOPED — they are
            lost when the CLI exits. For PERSISTENT scheduled tasks that
            survive across CLI sessions, use cloud schedule tools instead."#
        };
        get_or_init_channel();
        Self {
            tool_router: Self::tool_router(),
            instructions,
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[tool(
        name = "loop_create",
        description = "Create a CLI SESSION-SCOPED loop task (temporary, lost when CLI exits). The prompt is injected into the main agent conversation on each trigger, giving the task full agent capabilities (tool use, reasoning, multi-step work). If the agent is busy, execution is skipped. For PERSISTENT scheduled tasks that survive across sessions, use cloud schedule tools (createScheduledTask) instead. Schedules: 'every 5m', 'every 1h', 'every 30s', 'every hour at :15', or a 5-field cron expression like '*/10 * * * *'."
    )]
    async fn loop_create(
        &self,
        params: Parameters<LoopCreateParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let jobs = self.jobs.clone();
        let mut registry = jobs.lock().await;

        if registry.len() >= MAX_JOBS_PER_SESSION {
            return Err(ErrorData::new(
                ErrorCode::INVALID_REQUEST,
                format!(
                    "Maximum {} concurrent loop jobs per session. Remove some before adding new ones.",
                    MAX_JOBS_PER_SESSION
                ),
                None,
            ));
        }

        let parsed = parse_schedule(&params.schedule)
            .map_err(|e| ErrorData::new(ErrorCode::INVALID_PARAMS, e, None))?;

        let id = generate_id();
        let now = chrono::Utc::now();
        let cancel_token = tokio_util::sync::CancellationToken::new();

        let clear = params.clear.unwrap_or(false);
        let info = LoopJobInfo {
            id: id.clone(),
            schedule: params.schedule.clone(),
            prompt: params.prompt.clone(),
            cwd: params.cwd.clone(),
            clear,
            created_at: now.to_rfc3339(),
            last_run_at: None,
            next_run_at: None,
            run_count: 0,
            skip_count: 0,
            last_result: None,
            status: LoopJobStatus::Active,
        };

        let internal = InternalJob {
            info: info.clone(),
            cancel_token: cancel_token.clone(),
        };

        registry.insert(id.clone(), internal);
        drop(registry);

        spawn_job_executor(
            id.clone(),
            params.prompt,
            params.cwd,
            clear,
            parsed,
            cancel_token,
            jobs,
        );

        let result = serde_json::json!({
            "success": true,
            "message": format!("Loop task created: {}", params.schedule),
            "job": info,
            "note": "This job will inject the prompt into the main agent conversation on each trigger. If the agent is busy, the execution will be skipped. Use `loop_get` or `loop_list` MCP tools to check job status — there is no HTTP API."
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        name = "loop_list",
        description = "List all session-scoped loop tasks. Shows schedule, prompt, status, run count, skip count, last/next run time, and last result for each job."
    )]
    async fn loop_list(
        &self,
        _params: Parameters<LoopListParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let registry = self.jobs.lock().await;
        let jobs: Vec<&LoopJobInfo> = registry.values().map(|j| &j.info).collect();

        let result = serde_json::json!({
            "success": true,
            "jobs": jobs,
            "count": jobs.len(),
            "note": "All jobs are session-scoped. Prompts are injected into the main conversation when triggered."
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        name = "loop_remove",
        description = "Remove a session-scoped loop task by ID. The job is stopped and deleted immediately."
    )]
    async fn loop_remove(
        &self,
        params: Parameters<LoopRemoveParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let mut registry = self.jobs.lock().await;

        let job = registry.remove(&params.job_id).ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("Loop job not found: {}", params.job_id),
                None,
            )
        })?;

        job.cancel_token.cancel();

        let result = serde_json::json!({
            "success": true,
            "message": format!("Loop job {} removed", params.job_id),
            "removed_job": job.info
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        name = "loop_get",
        description = "Get detailed status of a specific loop task by ID."
    )]
    async fn loop_get(
        &self,
        params: Parameters<LoopGetParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let registry = self.jobs.lock().await;

        let job = registry.get(&params.job_id).ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("Loop job not found: {}", params.job_id),
                None,
            )
        })?;

        let result = serde_json::json!({
            "success": true,
            "job": job.info
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        name = "loop_pause",
        description = "Pause an active loop task. The job remains registered but stops firing prompts."
    )]
    async fn loop_pause(
        &self,
        params: Parameters<LoopPauseParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let mut registry = self.jobs.lock().await;

        let job = registry.get_mut(&params.job_id).ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("Loop job not found: {}", params.job_id),
                None,
            )
        })?;

        if job.info.status == LoopJobStatus::Paused {
            return Err(ErrorData::new(
                ErrorCode::INVALID_REQUEST,
                format!("Loop job {} is already paused", params.job_id),
                None,
            ));
        }

        job.info.status = LoopJobStatus::Paused;
        job.info.next_run_at = None;

        let result = serde_json::json!({
            "success": true,
            "message": format!("Loop job {} paused", params.job_id),
            "job": job.info
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(name = "loop_resume", description = "Resume a paused loop task.")]
    async fn loop_resume(
        &self,
        params: Parameters<LoopResumeParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let mut registry = self.jobs.lock().await;

        let job = registry.get_mut(&params.job_id).ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("Loop job not found: {}", params.job_id),
                None,
            )
        })?;

        if job.info.status == LoopJobStatus::Active {
            return Err(ErrorData::new(
                ErrorCode::INVALID_REQUEST,
                format!("Loop job {} is already active", params.job_id),
                None,
            ));
        }

        job.info.status = LoopJobStatus::Active;

        let result = serde_json::json!({
            "success": true,
            "message": format!("Loop job {} resumed", params.job_id),
            "job": job.info
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }
}

#[tool_handler]
impl ServerHandler for LoopServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(self.instructions.clone()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Schedule Parsing
// ---------------------------------------------------------------------------

enum ScheduleKind {
    Interval(u64),
    CronExpr(CronFields),
}

struct CronFields {
    minute: Vec<u32>,
    hour: Vec<u32>,
    day_of_month: Vec<u32>,
    month: Vec<u32>,
    day_of_week: Vec<u32>,
}

fn parse_schedule(schedule: &str) -> Result<ScheduleKind, String> {
    let s = schedule.trim().to_lowercase();

    let re_interval =
        regex::Regex::new(r"^every\s+(\d+)\s*(s|sec|seconds?|m|min|minutes?|h|hr|hours?)$")
            .unwrap();
    if let Some(caps) = re_interval.captures(&s) {
        let value: u64 = caps[1].parse().map_err(|_| "Invalid number".to_string())?;
        let unit = caps[2].chars().next().unwrap_or('m');
        let multiplier: u64 = match unit {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            _ => 60,
        };
        if value == 0 {
            return Err("Interval cannot be zero".into());
        }
        return Ok(ScheduleKind::Interval(value * multiplier));
    }

    let re_hourly = regex::Regex::new(r"^every\s+hour\s+at\s+:(\d{1,2})$").unwrap();
    if let Some(caps) = re_hourly.captures(&s) {
        let minute: u32 = caps[1].parse().map_err(|_| "Invalid minute".to_string())?;
        if minute > 59 {
            return Err(format!("Invalid minute: {}", minute));
        }
        return Ok(ScheduleKind::CronExpr(CronFields {
            minute: vec![minute],
            hour: (0..=23).collect(),
            day_of_month: (1..=31).collect(),
            month: (1..=12).collect(),
            day_of_week: (0..=6).collect(),
        }));
    }

    let parts: Vec<&str> = schedule.split_whitespace().collect();
    if parts.len() == 5 {
        return Ok(ScheduleKind::CronExpr(CronFields {
            minute: parse_cron_field(parts[0], 0, 59)?,
            hour: parse_cron_field(parts[1], 0, 23)?,
            day_of_month: parse_cron_field(parts[2], 1, 31)?,
            month: parse_cron_field(parts[3], 1, 12)?,
            day_of_week: parse_cron_field(parts[4], 0, 6)?,
        }));
    }

    Err(format!(
        "Unrecognized schedule \"{}\". Supported: \"every 5m\", \"every 1h\", \"every 30s\", \"every hour at :15\", or cron expression \"*/5 * * * *\"",
        schedule
    ))
}

fn parse_cron_field(field: &str, min: u32, max: u32) -> Result<Vec<u32>, String> {
    let mut values = std::collections::BTreeSet::new();

    for part in field.split(',') {
        let (range_str, step) = if let Some((r, s)) = part.split_once('/') {
            let step: u32 = s.parse().map_err(|_| format!("Invalid step: {}", s))?;
            if step == 0 {
                return Err("Step cannot be zero".into());
            }
            (r, step)
        } else {
            (part, 1)
        };

        if range_str == "*" {
            let mut i = min;
            while i <= max {
                values.insert(i);
                i += step;
            }
        } else if let Some((start_str, end_str)) = range_str.split_once('-') {
            let start: u32 = start_str
                .parse()
                .map_err(|_| format!("Invalid range start: {}", start_str))?;
            let end: u32 = end_str
                .parse()
                .map_err(|_| format!("Invalid range end: {}", end_str))?;
            if start < min || end > max {
                return Err(format!(
                    "Range {}-{} out of bounds [{}-{}]",
                    start, end, min, max
                ));
            }
            let mut i = start;
            while i <= end {
                values.insert(i);
                i += step;
            }
        } else {
            let val: u32 = range_str
                .parse()
                .map_err(|_| format!("Invalid value: {}", range_str))?;
            if val < min || val > max {
                return Err(format!("Value {} out of bounds [{}-{}]", val, min, max));
            }
            values.insert(val);
        }
    }

    Ok(values.into_iter().collect())
}

fn generate_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 4] = rng.gen();
    hex::encode(bytes)
}

fn get_next_cron_time(
    fields: &CronFields,
    after: chrono::DateTime<chrono::Utc>,
) -> chrono::DateTime<chrono::Utc> {
    use chrono::{Datelike, Timelike};
    let mut next = after + chrono::Duration::minutes(1);
    next = next.with_second(0).unwrap().with_nanosecond(0).unwrap();

    for _ in 0..525960 {
        if fields.month.contains(&(next.month()))
            && fields.day_of_month.contains(&next.day())
            && fields
                .day_of_week
                .contains(&next.weekday().num_days_from_sunday())
            && fields.hour.contains(&next.hour())
            && fields.minute.contains(&next.minute())
        {
            return next;
        }
        next += chrono::Duration::minutes(1);
    }

    after + chrono::Duration::hours(1)
}

// ---------------------------------------------------------------------------
// Job Executor — Prompt Injection
// ---------------------------------------------------------------------------

fn spawn_job_executor(
    job_id: String,
    prompt: String,
    cwd: Option<String>,
    clear: bool,
    schedule: ScheduleKind,
    cancel_token: tokio_util::sync::CancellationToken,
    jobs: JobRegistry,
) {
    tokio::spawn(async move {
        match schedule {
            ScheduleKind::Interval(secs) => {
                let duration = tokio::time::Duration::from_secs(secs);
                loop {
                    {
                        let mut reg = jobs.lock().await;
                        if let Some(job) = reg.get_mut(&job_id) {
                            let next = chrono::Utc::now() + chrono::Duration::seconds(secs as i64);
                            job.info.next_run_at = Some(next.to_rfc3339());
                        }
                    }

                    tokio::select! {
                        _ = tokio::time::sleep(duration) => {},
                        _ = cancel_token.cancelled() => break,
                    }

                    let should_run = {
                        let reg = jobs.lock().await;
                        reg.get(&job_id)
                            .map(|j| j.info.status == LoopJobStatus::Active)
                            .unwrap_or(false)
                    };

                    if should_run {
                        inject_prompt(&job_id, &prompt, cwd.as_deref(), clear, &jobs).await;
                    }
                }
            }
            ScheduleKind::CronExpr(fields) => {
                let fields = Arc::new(fields);
                loop {
                    let next_time = get_next_cron_time(&fields, chrono::Utc::now());
                    {
                        let mut reg = jobs.lock().await;
                        if let Some(job) = reg.get_mut(&job_id) {
                            job.info.next_run_at = Some(next_time.to_rfc3339());
                        }
                    }

                    let delay = (next_time - chrono::Utc::now())
                        .to_std()
                        .unwrap_or(std::time::Duration::from_secs(1));

                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {},
                        _ = cancel_token.cancelled() => break,
                    }

                    let should_run = {
                        let reg = jobs.lock().await;
                        reg.get(&job_id)
                            .map(|j| j.info.status == LoopJobStatus::Active)
                            .unwrap_or(false)
                    };

                    if should_run {
                        inject_prompt(&job_id, &prompt, cwd.as_deref(), clear, &jobs).await;
                    }
                }
            }
        }
    });
}

async fn inject_prompt(
    job_id: &str,
    prompt: &str,
    cwd: Option<&str>,
    clear: bool,
    jobs: &JobRegistry,
) {
    if is_agent_busy() {
        let mut reg = jobs.lock().await;
        if let Some(job) = reg.get_mut(job_id) {
            job.info.skip_count += 1;
            job.info.last_result = Some(LoopJobResult {
                success: false,
                summary: "Skipped: agent is busy processing another request".to_string(),
            });
        }
        return;
    }

    let schedule = {
        let reg = jobs.lock().await;
        reg.get(job_id)
            .map(|j| j.info.schedule.clone())
            .unwrap_or_default()
    };

    let channel = get_or_init_channel();
    let _ = channel.sender.send(LoopPromptEvent {
        job_id: job_id.to_string(),
        prompt: prompt.to_string(),
        cwd: cwd.map(|s| s.to_string()),
        schedule,
        clear,
    });

    let now = chrono::Utc::now().to_rfc3339();
    let truncated: String = prompt.chars().take(100).collect();
    let summary = if truncated.len() < prompt.len() {
        format!("Prompt injected: \"{truncated}...\"")
    } else {
        format!("Prompt injected: \"{prompt}\"")
    };

    let mut reg = jobs.lock().await;
    if let Some(job) = reg.get_mut(job_id) {
        job.info.last_run_at = Some(now);
        job.info.run_count += 1;
        job.info.last_result = Some(LoopJobResult {
            success: true,
            summary,
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_interval_seconds() {
        match parse_schedule("every 30s").unwrap() {
            ScheduleKind::Interval(secs) => assert_eq!(secs, 30),
            _ => panic!("Expected interval"),
        }
    }

    #[test]
    fn test_parse_interval_minutes() {
        match parse_schedule("every 5m").unwrap() {
            ScheduleKind::Interval(secs) => assert_eq!(secs, 300),
            _ => panic!("Expected interval"),
        }
    }

    #[test]
    fn test_parse_interval_hours() {
        match parse_schedule("every 2h").unwrap() {
            ScheduleKind::Interval(secs) => assert_eq!(secs, 7200),
            _ => panic!("Expected interval"),
        }
    }

    #[test]
    fn test_parse_hourly_at() {
        match parse_schedule("every hour at :15").unwrap() {
            ScheduleKind::CronExpr(fields) => {
                assert_eq!(fields.minute, vec![15]);
                assert_eq!(fields.hour.len(), 24);
            }
            _ => panic!("Expected cron expression"),
        }
    }

    #[test]
    fn test_parse_cron_expression() {
        match parse_schedule("*/10 * * * *").unwrap() {
            ScheduleKind::CronExpr(fields) => {
                assert_eq!(fields.minute, vec![0, 10, 20, 30, 40, 50]);
            }
            _ => panic!("Expected cron expression"),
        }
    }

    #[test]
    fn test_parse_cron_field_star() {
        let result = parse_cron_field("*", 0, 5).unwrap();
        assert_eq!(result, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_parse_cron_field_range() {
        let result = parse_cron_field("1-3", 0, 5).unwrap();
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[test]
    fn test_parse_cron_field_step() {
        let result = parse_cron_field("*/2", 0, 6).unwrap();
        assert_eq!(result, vec![0, 2, 4, 6]);
    }

    #[test]
    fn test_parse_cron_field_list() {
        let result = parse_cron_field("1,3,5", 0, 6).unwrap();
        assert_eq!(result, vec![1, 3, 5]);
    }

    #[test]
    fn test_parse_invalid_schedule() {
        assert!(parse_schedule("invalid").is_err());
    }

    #[test]
    fn test_parse_zero_interval() {
        assert!(parse_schedule("every 0s").is_err());
    }

    #[test]
    fn test_generate_id_uniqueness() {
        let id1 = generate_id();
        let id2 = generate_id();
        assert_ne!(id1, id2);
        assert_eq!(id1.len(), 8);
    }

    #[test]
    fn test_parse_cron_range_with_step() {
        let result = parse_cron_field("1-10/3", 0, 59).unwrap();
        assert_eq!(result, vec![1, 4, 7, 10]);
    }

    #[test]
    fn test_parse_cron_out_of_bounds() {
        assert!(parse_cron_field("60", 0, 59).is_err());
    }
}
