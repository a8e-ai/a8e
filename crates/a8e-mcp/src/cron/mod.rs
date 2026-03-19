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
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

const MAX_JOBS_PER_SESSION: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJobInfo {
    pub id: String,
    pub schedule: String,
    pub command: String,
    pub cwd: Option<String>,
    pub created_at: String,
    pub last_run_at: Option<String>,
    pub next_run_at: Option<String>,
    pub run_count: u64,
    pub last_result: Option<CronJobResult>,
    pub status: CronJobStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJobResult {
    pub success: bool,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CronJobStatus {
    Active,
    Paused,
}

struct InternalJob {
    info: CronJobInfo,
    cancel_token: tokio_util::sync::CancellationToken,
}

type JobRegistry = Arc<Mutex<HashMap<String, InternalJob>>>;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CronCreateParams {
    /// Schedule expression. Supported formats:
    /// - "every 5m", "every 1h", "every 30s" (fixed interval)
    /// - "every hour at :15" (hourly at specific minute)
    /// - Cron expression: "*/10 * * * *" (every 10 min), "0 9 * * 1" (Monday 9AM)
    pub schedule: String,
    /// Shell command to execute on each trigger
    pub command: String,
    /// Working directory for the command (optional, defaults to current dir)
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CronListParams {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CronRemoveParams {
    /// The ID of the cron job to remove
    pub job_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CronGetParams {
    /// The ID of the cron job
    pub job_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CronPauseParams {
    /// The ID of the cron job to pause
    pub job_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CronResumeParams {
    /// The ID of the cron job to resume
    pub job_id: String,
}

#[derive(Clone)]
pub struct CronServer {
    tool_router: ToolRouter<Self>,
    instructions: String,
    jobs: JobRegistry,
}

impl Default for CronServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router(router = tool_router)]
impl CronServer {
    pub fn new() -> Self {
        let instructions = formatdoc! {r#"
            Session-scoped scheduled task (cron) management.
            Create, list, pause, resume, and remove recurring shell commands
            that run on a schedule within the current CLI session.
            All jobs are automatically cleaned up when the session ends.
            Use this to set up periodic tasks like health checks, log rotation,
            data fetching, file syncing, or any recurring automation."#
        };
        Self {
            tool_router: Self::tool_router(),
            instructions,
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[tool(
        name = "cron_create",
        description = "Create a session-scoped scheduled task (cron job). The job runs repeatedly on the given schedule until removed or the session ends. Schedules: 'every 5m', 'every 1h', 'every 30s', 'every hour at :15', or a 5-field cron expression like '*/10 * * * *'. Commands are executed through the shell."
    )]
    async fn cron_create(
        &self,
        params: Parameters<CronCreateParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let jobs = self.jobs.clone();
        let mut registry = jobs.lock().await;

        if registry.len() >= MAX_JOBS_PER_SESSION {
            return Err(ErrorData::new(
                ErrorCode::INVALID_REQUEST,
                format!(
                    "Maximum {} concurrent cron jobs per session. Remove some before adding new ones.",
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

        let info = CronJobInfo {
            id: id.clone(),
            schedule: params.schedule.clone(),
            command: params.command.clone(),
            cwd: params.cwd.clone(),
            created_at: now.to_rfc3339(),
            last_run_at: None,
            next_run_at: None,
            run_count: 0,
            last_result: None,
            status: CronJobStatus::Active,
        };

        let internal = InternalJob {
            info: info.clone(),
            cancel_token: cancel_token.clone(),
        };

        registry.insert(id.clone(), internal);
        drop(registry);

        spawn_job_executor(
            id.clone(),
            params.command,
            params.cwd,
            parsed,
            cancel_token,
            jobs,
        );

        let result = serde_json::json!({
            "success": true,
            "message": format!("Cron job created: {}", params.schedule),
            "job": info,
            "note": "This job is session-scoped and will be removed when the CLI session ends."
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        name = "cron_list",
        description = "List all session-scoped cron jobs. Shows schedule, status, run count, last/next run time, and last result for each job."
    )]
    async fn cron_list(
        &self,
        _params: Parameters<CronListParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let registry = self.jobs.lock().await;
        let jobs: Vec<&CronJobInfo> = registry.values().map(|j| &j.info).collect();

        let result = serde_json::json!({
            "success": true,
            "jobs": jobs,
            "count": jobs.len(),
            "note": "All jobs are session-scoped. They will be removed when the CLI session ends."
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        name = "cron_remove",
        description = "Remove a session-scoped cron job by ID. The job is stopped and deleted immediately."
    )]
    async fn cron_remove(
        &self,
        params: Parameters<CronRemoveParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let mut registry = self.jobs.lock().await;

        let job = registry.remove(&params.job_id).ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("Cron job not found: {}", params.job_id),
                None,
            )
        })?;

        job.cancel_token.cancel();

        let result = serde_json::json!({
            "success": true,
            "message": format!("Cron job {} removed", params.job_id),
            "removed_job": job.info
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        name = "cron_get",
        description = "Get detailed status of a specific cron job by ID."
    )]
    async fn cron_get(
        &self,
        params: Parameters<CronGetParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let registry = self.jobs.lock().await;

        let job = registry.get(&params.job_id).ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("Cron job not found: {}", params.job_id),
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
        name = "cron_pause",
        description = "Pause an active cron job. The job remains registered but stops executing."
    )]
    async fn cron_pause(
        &self,
        params: Parameters<CronPauseParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let mut registry = self.jobs.lock().await;

        let job = registry.get_mut(&params.job_id).ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("Cron job not found: {}", params.job_id),
                None,
            )
        })?;

        if job.info.status == CronJobStatus::Paused {
            return Err(ErrorData::new(
                ErrorCode::INVALID_REQUEST,
                format!("Cron job {} is already paused", params.job_id),
                None,
            ));
        }

        job.info.status = CronJobStatus::Paused;
        job.info.next_run_at = None;

        let result = serde_json::json!({
            "success": true,
            "message": format!("Cron job {} paused", params.job_id),
            "job": job.info
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(name = "cron_resume", description = "Resume a paused cron job.")]
    async fn cron_resume(
        &self,
        params: Parameters<CronResumeParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let mut registry = self.jobs.lock().await;

        let job = registry.get_mut(&params.job_id).ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("Cron job not found: {}", params.job_id),
                None,
            )
        })?;

        if job.info.status == CronJobStatus::Active {
            return Err(ErrorData::new(
                ErrorCode::INVALID_REQUEST,
                format!("Cron job {} is already active", params.job_id),
                None,
            ));
        }

        job.info.status = CronJobStatus::Active;

        let result = serde_json::json!({
            "success": true,
            "message": format!("Cron job {} resumed", params.job_id),
            "job": job.info
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }
}

#[tool_handler]
impl ServerHandler for CronServer {
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
    Cron(CronFields),
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
        return Ok(ScheduleKind::Cron(CronFields {
            minute: vec![minute],
            hour: (0..=23).collect(),
            day_of_month: (1..=31).collect(),
            month: (1..=12).collect(),
            day_of_week: (0..=6).collect(),
        }));
    }

    let parts: Vec<&str> = schedule.split_whitespace().collect();
    if parts.len() == 5 {
        return Ok(ScheduleKind::Cron(CronFields {
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

fn spawn_job_executor(
    job_id: String,
    command: String,
    cwd: Option<String>,
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
                            .map(|j| j.info.status == CronJobStatus::Active)
                            .unwrap_or(false)
                    };

                    if should_run {
                        execute_and_update(&job_id, &command, cwd.as_deref(), &jobs).await;
                    }
                }
            }
            ScheduleKind::Cron(fields) => {
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
                            .map(|j| j.info.status == CronJobStatus::Active)
                            .unwrap_or(false)
                    };

                    if should_run {
                        execute_and_update(&job_id, &command, cwd.as_deref(), &jobs).await;
                    }
                }
            }
        }
    });
}

async fn execute_and_update(job_id: &str, command: &str, cwd: Option<&str>, jobs: &JobRegistry) {
    let result = execute_shell_command(command, cwd).await;
    let now = chrono::Utc::now().to_rfc3339();

    let mut reg = jobs.lock().await;
    if let Some(job) = reg.get_mut(job_id) {
        job.info.last_run_at = Some(now);
        job.info.run_count += 1;
        job.info.last_result = Some(result);
    }
}

async fn execute_shell_command(command: &str, cwd: Option<&str>) -> CronJobResult {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| {
        if cfg!(windows) {
            "cmd".to_string()
        } else {
            "bash".to_string()
        }
    });

    let shell_arg = if cfg!(windows) { "/c" } else { "-c" };

    let mut cmd = tokio::process::Command::new(&shell);
    cmd.arg(shell_arg).arg(command);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdin(std::process::Stdio::null());

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    match tokio::time::timeout(tokio::time::Duration::from_secs(300), cmd.output()).await {
        Ok(Ok(output)) => {
            let success = output.status.success();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let summary = if success {
                let s = stdout.trim();
                if s.is_empty() {
                    "OK".to_string()
                } else {
                    s.chars().take(200).collect()
                }
            } else {
                let s = stderr.trim();
                if s.is_empty() {
                    format!("Exit code: {}", output.status.code().unwrap_or(-1))
                } else {
                    s.chars().take(200).collect()
                }
            };
            CronJobResult { success, summary }
        }
        Ok(Err(e)) => CronJobResult {
            success: false,
            summary: format!("Failed to execute: {}", e),
        },
        Err(_) => CronJobResult {
            success: false,
            summary: "Command timed out after 300s".to_string(),
        },
    }
}

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
            ScheduleKind::Cron(fields) => {
                assert_eq!(fields.minute, vec![15]);
                assert_eq!(fields.hour.len(), 24);
            }
            _ => panic!("Expected cron"),
        }
    }

    #[test]
    fn test_parse_cron_expression() {
        match parse_schedule("*/10 * * * *").unwrap() {
            ScheduleKind::Cron(fields) => {
                assert_eq!(fields.minute, vec![0, 10, 20, 30, 40, 50]);
            }
            _ => panic!("Expected cron"),
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
