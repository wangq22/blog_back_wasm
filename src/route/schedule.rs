//! Live class-aware todo scheduling.
//!
//! The class data is intentionally small and lives in code because the source
//! timetable is a temporary reference. The original image is never uploaded
//! or served. The same weekly slots are used by the API when it checks AI plans
//! and chooses a deterministic fallback.

use axum::{extract::Path, http::StatusCode, response::IntoResponse, Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use time::{
    format_description::well_known::Rfc3339, Date, Duration, OffsetDateTime, PrimitiveDateTime,
    Time, UtcOffset, Weekday,
};
use worker::{send::SendWrapper, Ai, D1Database, Date as WorkerDate, Env};

const DEFAULT_AI_MODEL: &str = "@cf/deepseek-ai/deepseek-v4-flash-0731";
const MAX_TASK_MINUTES: i32 = 240;

#[derive(Clone, Copy)]
struct ClassSlot {
    weekday: u8,
    code: &'static str,
    start_minutes: i32,
    end_minutes: i32,
}

// Monday = 0. This is the normalized form of class_schedule.jpg.
const CLASS_SCHEDULE: [ClassSlot; 12] = [
    ClassSlot {
        weekday: 0,
        code: "STATS 250-100",
        start_minutes: 10 * 60,
        end_minutes: 11 * 60 + 30,
    },
    ClassSlot {
        weekday: 0,
        code: "STATS 250-103",
        start_minutes: 13 * 60,
        end_minutes: 14 * 60 + 30,
    },
    ClassSlot {
        weekday: 0,
        code: "EECS 280-004",
        start_minutes: 14 * 60 + 30,
        end_minutes: 16 * 60,
    },
    ClassSlot {
        weekday: 1,
        code: "EECS 280-012",
        start_minutes: 8 * 60 + 30,
        end_minutes: 10 * 60 + 30,
    },
    ClassSlot {
        weekday: 1,
        code: "EECS 203-005",
        start_minutes: 12 * 60,
        end_minutes: 13 * 60 + 30,
    },
    ClassSlot {
        weekday: 1,
        code: "ASTRO 102-006",
        start_minutes: 14 * 60 + 30,
        end_minutes: 16 * 60,
    },
    ClassSlot {
        weekday: 2,
        code: "ASTRO 102-010",
        start_minutes: 9 * 60,
        end_minutes: 10 * 60,
    },
    ClassSlot {
        weekday: 2,
        code: "STATS 250-100",
        start_minutes: 10 * 60,
        end_minutes: 11 * 60 + 30,
    },
    ClassSlot {
        weekday: 2,
        code: "EECS 280-004",
        start_minutes: 14 * 60 + 30,
        end_minutes: 16 * 60,
    },
    ClassSlot {
        weekday: 3,
        code: "EECS 203-005",
        start_minutes: 12 * 60,
        end_minutes: 13 * 60 + 30,
    },
    ClassSlot {
        weekday: 3,
        code: "ASTRO 102-006",
        start_minutes: 14 * 60 + 30,
        end_minutes: 16 * 60,
    },
    ClassSlot {
        weekday: 4,
        code: "EECS 203-051",
        start_minutes: 10 * 60,
        end_minutes: 11 * 60 + 30,
    },
];

#[derive(Debug, Deserialize)]
pub struct TaskCreateDTO {
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub deadline: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReviewDTO {
    pub outcome: String,
    #[serde(default)]
    pub spent_minutes: i32,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub next_start: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReactionDTO {
    pub emoji: String,
}

#[derive(Clone, Copy)]
struct Interval {
    start: OffsetDateTime,
    end: OffsetDateTime,
}

struct Plan {
    start: OffsetDateTime,
    end: OffsetDateTime,
    minutes: i32,
    reason: String,
    used_ai: bool,
}

fn now_utc() -> OffsetDateTime {
    // `time::OffsetDateTime::now_utc()` reaches `SystemTime::now()`, which
    // panics in Cloudflare Workers' wasm runtime. Workers exposes the clock
    // through the JavaScript Date API instead.
    OffsetDateTime::from_unix_timestamp_nanos(WorkerDate::now().as_millis() as i128 * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

fn now_iso() -> String {
    now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn parse_iso(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value.trim(), &Rfc3339).ok()
}

fn format_iso(value: OffsetDateTime) -> String {
    value
        .to_offset(UtcOffset::UTC)
        .format(&Rfc3339)
        .unwrap_or_else(|_| now_iso())
}

fn parse_utc_offset(env: &Env) -> UtcOffset {
    let raw = env
        .var("SCHEDULE_UTC_OFFSET")
        .ok()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-04:00".to_string());
    let trimmed = raw.trim();
    let sign = if trimmed.starts_with('-') { -1 } else { 1 };
    let numbers = trimmed.trim_start_matches(|ch| ch == '+' || ch == '-');
    let mut parts = numbers.split(':');
    let hours = parts
        .next()
        .and_then(|value| value.parse::<i8>().ok())
        .unwrap_or(4);
    let minutes = parts
        .next()
        .and_then(|value| value.parse::<i8>().ok())
        .unwrap_or(0);
    UtcOffset::from_hms(sign * hours, sign * minutes, 0)
        .unwrap_or_else(|_| UtcOffset::from_hms(-4, 0, 0).unwrap_or(UtcOffset::UTC))
}

fn ai_model(env: &Env) -> String {
    env.var("SCHEDULE_AI_MODEL")
        .ok()
        .map(|value| value.to_string())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_AI_MODEL.to_string())
}

fn value_string(row: &Value, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn value_i64(row: &Value, key: &str) -> i64 {
    row.get(key)
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().map(|number| number as i64))
        })
        .unwrap_or_default()
}

fn value_i32(row: &Value, key: &str) -> i32 {
    value_i64(row, key).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

fn row_interval(row: &Value) -> Option<Interval> {
    Some(Interval {
        start: parse_iso(&value_string(row, "scheduled_start"))?,
        end: parse_iso(&value_string(row, "scheduled_end"))?,
    })
}

fn overlaps(start: OffsetDateTime, end: OffsetDateTime, interval: Interval) -> bool {
    start < interval.end && end > interval.start
}

fn date_weekday(date: Date) -> u8 {
    match date.weekday() {
        Weekday::Monday => 0,
        Weekday::Tuesday => 1,
        Weekday::Wednesday => 2,
        Weekday::Thursday => 3,
        Weekday::Friday => 4,
        Weekday::Saturday => 5,
        Weekday::Sunday => 6,
    }
}

fn local_time(date: Date, minutes: i32, offset: UtcOffset) -> OffsetDateTime {
    let primitive =
        PrimitiveDateTime::new(date, Time::MIDNIGHT) + Duration::minutes(minutes as i64);
    primitive.assume_offset(offset).to_offset(UtcOffset::UTC)
}

fn class_intervals(env: &Env, days: i64) -> Vec<Interval> {
    let offset = parse_utc_offset(env);
    let local_today = now_utc().to_offset(offset).date();
    let mut intervals = Vec::new();
    for day_offset in 0..days {
        let date = local_today + Duration::days(day_offset);
        let weekday = date_weekday(date);
        for class in CLASS_SCHEDULE
            .iter()
            .filter(|class| class.weekday == weekday)
        {
            intervals.push(Interval {
                start: local_time(date, class.start_minutes, offset),
                end: local_time(date, class.end_minutes, offset),
            });
        }
    }
    intervals
}

async fn active_task_intervals(db: &D1Database) -> Vec<Interval> {
    let rows = match db
        .prepare(
            "SELECT scheduled_start, scheduled_end
             FROM schedule_tasks
             WHERE status IN ('planned', 'in_progress')",
        )
        .all()
        .await
    {
        Ok(result) => result.results::<Value>().unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    rows.iter().filter_map(row_interval).collect()
}

fn round_up_to_half_hour(minutes: i32) -> i32 {
    ((minutes + 29) / 30) * 30
}

fn next_available_slot(
    env: &Env,
    duration_minutes: i32,
    deadline: Option<OffsetDateTime>,
    existing: &[Interval],
) -> Plan {
    let offset = parse_utc_offset(env);
    let now = now_utc();
    let local_now = now.to_offset(offset);
    let duration = duration_minutes.clamp(30, MAX_TASK_MINUTES);
    let class_busy = class_intervals(env, 21);

    for day_offset in 0..21_i64 {
        let date = local_now.date() + Duration::days(day_offset);
        let current_minutes =
            local_now.time().hour() as i32 * 60 + local_now.time().minute() as i32;
        let first = if day_offset == 0 {
            round_up_to_half_hour(current_minutes + 15).max(8 * 60)
        } else {
            8 * 60
        };
        let last = 22 * 60 - duration;
        if last < first {
            continue;
        }
        let mut start_minutes = first;
        while start_minutes <= last {
            let start = local_time(date, start_minutes, offset);
            let end = start + Duration::minutes(duration as i64);
            let is_busy = class_busy
                .iter()
                .chain(existing.iter())
                .any(|busy| overlaps(start, end, *busy));
            let misses_deadline = deadline.map(|limit| end > limit).unwrap_or(false);
            if start > now && !is_busy && !misses_deadline {
                return Plan {
                    start,
                    end,
                    minutes: duration,
                    reason: "The next open slot clears your recurring classes and active tasks."
                        .to_string(),
                    used_ai: false,
                };
            }
            start_minutes += 30;
        }
    }

    let start = now + Duration::minutes(15);
    Plan {
        start,
        end: start + Duration::minutes(duration as i64),
        minutes: duration,
        reason: "No open slot was found in the next three weeks; this is a provisional fallback."
            .to_string(),
        used_ai: false,
    }
}

fn minutes_label(minutes: i32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn class_context(env: &Env) -> String {
    let offset = parse_utc_offset(env);
    let local_today = now_utc().to_offset(offset).date();
    let mut lines = Vec::new();
    for day_offset in 0..14_i64 {
        let date = local_today + Duration::days(day_offset);
        let weekday = date_weekday(date);
        for class in CLASS_SCHEDULE
            .iter()
            .filter(|class| class.weekday == weekday)
        {
            lines.push(format!(
                "{} {} {}-{}",
                date,
                class.code,
                minutes_label(class.start_minutes),
                minutes_label(class.end_minutes)
            ));
        }
    }
    if lines.is_empty() {
        "No class blocks in the planning window.".to_string()
    } else {
        lines.join("\n")
    }
}

async fn learning_note(db: &D1Database) -> Option<String> {
    match db
        .prepare("SELECT content FROM schedule_learning WHERE _id = 1")
        .first::<Value>(None)
        .await
    {
        Ok(Some(row)) => row
            .get("content")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .filter(|content| !content.trim().is_empty()),
        _ => None,
    }
}

fn extract_ai_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    if let Some(text) = value.get("response").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = value.get("output").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = value.get("output_text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn extract_json_object(text: &str) -> Option<Value> {
    let cleaned = text.replace("```json", "").replace("```", "");
    let trimmed = cleaned.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    serde_json::from_str::<Value>(&trimmed[start..=end]).ok()
}

async fn run_ai(env: &Env, prompt: String, max_tokens: u32) -> Option<String> {
    let ai: Ai = env.ai("AI").ok()?;
    let model = ai_model(env);
    let input = json!({
        "messages": [
            { "role": "system", "content": "You are a careful personal time planner. Follow the requested output format exactly." },
            { "role": "user", "content": prompt }
        ],
        "max_tokens": max_tokens,
        "temperature": 0.1,
    });
    let response: Value = ai.run(model, input).await.ok()?;
    extract_ai_text(&response)
}

async fn ai_plan(
    env: &Env,
    task: &TaskCreateDTO,
    deadline: Option<OffsetDateTime>,
    existing: &[Interval],
    learning: Option<String>,
) -> Option<Plan> {
    let timezone = task.timezone.as_deref().unwrap_or("America/Detroit");
    let busy = existing
        .iter()
        .map(|interval| {
            format!(
                "{} to {}",
                format_iso(interval.start),
                format_iso(interval.end)
            )
        })
        .collect::<Vec<_>>();
    let prompt = format!(
        "Plan this task for one person.\n\nTASK TITLE:\n{}\n\nTASK NOTES:\n{}\n\nTIMEZONE LABEL:\n{}\nCURRENT UTC:\n{}\nFIXED CLASS BLOCKS (local dates and times):\n{}\nACTIVE TASK BLOCKS (UTC):\n{}\nLEARNED SCHEDULING NOTES FROM EARLIER REVIEWS:\n{}\nDEADLINE (UTC, if any):\n{}\n\nChoose a realistic uninterrupted block outside every class and active task. Use 30-minute increments, prefer 08:00-22:00 local time, keep the task between 30 and 240 minutes, and honor the deadline. Return ONLY one JSON object with exactly these keys: duration_minutes (integer), start_at (RFC3339 UTC string), end_at (RFC3339 UTC string), reason (one short sentence).",
        task.title.trim(),
        task.notes.trim(),
        timezone,
        now_iso(),
        class_context(env),
        if busy.is_empty() { "None".to_string() } else { busy.join("\n") },
        learning.unwrap_or_else(|| "No learned notes yet.".to_string()),
        deadline.map(format_iso).unwrap_or_else(|| "None".to_string()),
    );
    let text = run_ai(env, prompt, 500).await?;
    let object = extract_json_object(&text)?;
    let start = parse_iso(object.get("start_at")?.as_str()?)?;
    let end = parse_iso(object.get("end_at")?.as_str()?)?;
    let duration = (end - start).whole_minutes() as i32;
    if end <= start
        || duration < 30
        || duration > MAX_TASK_MINUTES
        || start <= now_utc() - Duration::minutes(5)
        || deadline.map(|limit| end > limit).unwrap_or(false)
    {
        return None;
    }
    let class_busy = class_intervals(env, 21);
    if class_busy
        .iter()
        .chain(existing.iter())
        .any(|busy| overlaps(start, end, *busy))
    {
        return None;
    }
    Some(Plan {
        start,
        end,
        minutes: duration,
        reason: object
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("AI selected an open block around your fixed commitments.")
            .chars()
            .take(240)
            .collect(),
        used_ai: true,
    })
}

async fn plan_task(env: &Env, task: &TaskCreateDTO, deadline: Option<OffsetDateTime>) -> Plan {
    let db = match env.d1("DB") {
        Ok(db) => db,
        Err(_) => return next_available_slot(env, 60, deadline, &[]),
    };
    let existing = active_task_intervals(&db).await;
    let learning = learning_note(&db).await;
    ai_plan(env, task, deadline, &existing, learning)
        .await
        .unwrap_or_else(|| next_available_slot(env, 60, deadline, &existing))
}

async fn public_task_with_reactions(db: &D1Database, row: &Value) -> Value {
    let id = value_i64(row, "_id");
    let reaction_rows = match db
        .prepare("SELECT emoji, count FROM schedule_reactions WHERE task_id = ?1 ORDER BY count DESC, emoji ASC")
        .bind(&[id.into()])
    {
        Ok(statement) => statement
            .all()
            .await
            .ok()
            .and_then(|result| result.results::<Value>().ok())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    json!({
        "id": id,
        "title": value_string(row, "title"),
        "scheduled_start": value_string(row, "scheduled_start"),
        "scheduled_end": value_string(row, "scheduled_end"),
        "estimated_minutes": value_i32(row, "estimated_minutes"),
        "status": value_string(row, "status"),
        "ai_reason": value_string(row, "ai_reason"),
        "reactions": reaction_rows,
    })
}

async fn promote_due_tasks(env: &Env) -> Result<(), String> {
    let db = env.d1("DB").map_err(|error| error.to_string())?;
    let now = now_iso();
    db.prepare(
        "INSERT OR IGNORE INTO schedule_task_reviews (task_id, status, prompted_at, summary)
         SELECT _id, 'pending', ?1, ''
         FROM schedule_tasks
         WHERE status IN ('planned', 'in_progress')
           AND scheduled_end <= ?1
           AND NOT EXISTS (
             SELECT 1 FROM schedule_task_reviews r
             WHERE r.task_id = schedule_tasks._id AND r.status = 'pending'
           )",
    )
    .bind(&[now.clone().into()])
    .map_err(|error| error.to_string())?
    .run()
    .await
    .map_err(|error| error.to_string())?;
    db.prepare(
        "UPDATE schedule_tasks
         SET status = 'awaiting_review', updated_at = ?1
         WHERE status IN ('planned', 'in_progress') AND scheduled_end <= ?1",
    )
    .bind(&[now.into()])
    .map_err(|error| error.to_string())?
    .run()
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[worker::send]
pub async fn list_public_tasks(Extension(env): Extension<SendWrapper<Env>>) -> impl IntoResponse {
    let _ = promote_due_tasks(&env).await;
    let db = match env.d1("DB") {
        Ok(db) => db,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error.to_string() })),
            )
                .into_response()
        }
    };
    let rows = match db
        .prepare(
            "SELECT _id, title, scheduled_start, scheduled_end, estimated_minutes, status, ai_reason
             FROM schedule_tasks
             WHERE status IN ('planned', 'in_progress', 'awaiting_review')
             ORDER BY scheduled_start ASC",
        )
        .all()
        .await
    {
        Ok(result) => result.results::<Value>().unwrap_or_default(),
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": error.to_string() }))).into_response(),
    };
    let mut tasks = Vec::with_capacity(rows.len());
    for row in rows.iter() {
        tasks.push(public_task_with_reactions(&db, row).await);
    }
    (StatusCode::OK, Json(tasks)).into_response()
}

#[worker::send]
pub async fn add_task(
    Extension(env): Extension<SendWrapper<Env>>,
    Json(payload): Json<TaskCreateDTO>,
) -> impl IntoResponse {
    let title = payload.title.trim();
    if title.is_empty() || title.len() > 160 || payload.notes.len() > 1000 {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "title is required (160 chars max) and notes must be 1000 chars max" }))).into_response();
    }
    let deadline = match payload.deadline.as_deref() {
        Some(value) if !value.trim().is_empty() => match parse_iso(value) {
            Some(date) => Some(date),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "deadline must be an RFC3339 timestamp" })),
                )
                    .into_response()
            }
        },
        _ => None,
    };
    let plan = plan_task(&env, &payload, deadline).await;
    let now = now_iso();
    let model = ai_model(&env);
    let db = match env.d1("DB") {
        Ok(db) => db,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error.to_string() })),
            )
                .into_response()
        }
    };
    let result = match db
        .prepare(
            "INSERT INTO schedule_tasks
             (title, notes, scheduled_start, scheduled_end, estimated_minutes, status, ai_reason, ai_model, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'planned', ?6, ?7, ?8, ?8)",
        )
        .bind(&[
            title.to_string().into(),
            payload.notes.trim().to_string().into(),
            format_iso(plan.start).into(),
            format_iso(plan.end).into(),
            plan.minutes.into(),
            plan.reason.clone().into(),
            model.clone().into(),
            now.into(),
        ])
        .unwrap()
        .run()
        .await
    {
        Ok(result) => result,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": error.to_string() }))).into_response(),
    };
    let id = result
        .meta()
        .ok()
        .flatten()
        .and_then(|meta| meta.last_row_id)
        .unwrap_or_default();
    let row = db
        .prepare("SELECT _id, title, scheduled_start, scheduled_end, estimated_minutes, status, ai_reason FROM schedule_tasks WHERE _id = ?1")
        .bind(&[id.into()])
        .unwrap()
        .first::<Value>(None)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| json!({ "_id": id, "title": title }));
    let task = public_task_with_reactions(&db, &row).await;
    (
        StatusCode::OK,
        Json(json!({
            "task": task,
            "ai": {
                "model": model,
                "used": plan.used_ai,
                "reason": plan.reason,
            }
        })),
    )
        .into_response()
}

#[worker::send]
pub async fn add_reaction(
    Extension(env): Extension<SendWrapper<Env>>,
    Path(task_id): Path<i64>,
    Json(payload): Json<ReactionDTO>,
) -> impl IntoResponse {
    const ALLOWED: [&str; 8] = ["👏", "🔥", "💡", "🚀", "☕", "❤️", "👍", "🎉"];
    if !ALLOWED.contains(&payload.emoji.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "unsupported emoji" })),
        )
            .into_response();
    }
    let db = match env.d1("DB") {
        Ok(db) => db,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error.to_string() })),
            )
                .into_response()
        }
    };
    let exists = match db
        .prepare("SELECT _id FROM schedule_tasks WHERE _id = ?1 AND status IN ('planned', 'in_progress', 'awaiting_review')")
        .bind(&[task_id.into()])
    {
        Ok(statement) => statement.first::<Value>(None).await.ok().flatten().is_some(),
        Err(_) => false,
    };
    if !exists {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "task not found" })),
        )
            .into_response();
    }
    if let Err(error) = db
        .prepare(
            "INSERT INTO schedule_reactions (task_id, emoji, count) VALUES (?1, ?2, 1)
             ON CONFLICT(task_id, emoji) DO UPDATE SET count = count + 1",
        )
        .bind(&[task_id.into(), payload.emoji.into()])
        .unwrap()
        .run()
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response();
    }
    let reaction_rows = db
        .prepare("SELECT emoji, count FROM schedule_reactions WHERE task_id = ?1 ORDER BY count DESC, emoji ASC")
        .bind(&[task_id.into()])
        .unwrap()
        .all()
        .await
        .ok()
        .and_then(|result| result.results::<Value>().ok())
        .unwrap_or_default();
    (StatusCode::OK, Json(json!({ "reactions": reaction_rows }))).into_response()
}

#[worker::send]
pub async fn get_reviews(Extension(env): Extension<SendWrapper<Env>>) -> impl IntoResponse {
    let _ = promote_due_tasks(&env).await;
    let db = match env.d1("DB") {
        Ok(db) => db,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error.to_string() })),
            )
                .into_response()
        }
    };
    let reviews = db
        .prepare(
            "SELECT r._id, r.task_id, r.prompted_at, r.status, t.title, t.scheduled_start, t.scheduled_end, t.estimated_minutes
             FROM schedule_task_reviews r
             JOIN schedule_tasks t ON t._id = r.task_id
             WHERE r.status = 'pending'
             ORDER BY r.prompted_at ASC",
        )
        .all()
        .await
        .ok()
        .and_then(|result| result.results::<Value>().ok())
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            json!({
                "id": value_i64(&row, "_id"),
                "task_id": value_i64(&row, "task_id"),
                "title": value_string(&row, "title"),
                "scheduled_start": value_string(&row, "scheduled_start"),
                "scheduled_end": value_string(&row, "scheduled_end"),
                "estimated_minutes": value_i32(&row, "estimated_minutes"),
                "prompted_at": value_string(&row, "prompted_at"),
                "status": value_string(&row, "status"),
            })
        })
        .collect::<Vec<_>>();
    let learning = learning_note(&db)
        .await
        .map(|content| json!({ "content": content }));
    (
        StatusCode::OK,
        Json(json!({ "reviews": reviews, "learning": learning })),
    )
        .into_response()
}

async fn refresh_learning(env: &Env, db: &D1Database) -> Option<Value> {
    let rows = db
        .prepare(
            "SELECT t.title, r.outcome, r.spent_minutes, r.summary, r.reviewed_at, t.estimated_minutes
             FROM schedule_task_reviews r
             JOIN schedule_tasks t ON t._id = r.task_id
             WHERE r.status IN ('completed', 'deferred', 'skipped')
             ORDER BY r.reviewed_at DESC LIMIT 20",
        )
        .all()
        .await
        .ok()
        .and_then(|result| result.results::<Value>().ok())
        .unwrap_or_default();
    if rows.is_empty() {
        return learning_note(db)
            .await
            .map(|content| json!({ "content": content }));
    }
    let history = rows
        .iter()
        .map(|row| {
            format!(
                "Task: {} | outcome: {} | estimated: {} min | spent: {} min | note: {}",
                value_string(row, "title"),
                value_string(row, "outcome"),
                value_i32(row, "estimated_minutes"),
                value_i32(row, "spent_minutes"),
                value_string(row, "summary"),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let old = learning_note(db)
        .await
        .unwrap_or_else(|| "No previous learning note.".to_string());
    let prompt = format!(
        "Review these recent scheduling outcomes and write a compact memory for the next task scheduler. Keep it to 2-4 sentences, mention useful patterns in estimates, time of day, class days, or follow-through, and give one concrete adjustment. Do not mention private implementation details. Return only the memory text.\n\nCURRENT MEMORY:\n{}\n\nRECENT OUTCOMES:\n{}",
        old, history
    );
    let content = run_ai(env, prompt, 300)
        .await
        .map(|text| text.trim().trim_matches('"').chars().take(1200).collect::<String>())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| {
            let latest = &rows[0];
            format!(
                "Recent outcomes show {} spent {} minutes against a {} minute estimate. Keep checking the estimate against the short reflection before placing similar work.",
                value_string(latest, "outcome"),
                value_i32(latest, "spent_minutes"),
                value_i32(latest, "estimated_minutes")
            )
        });
    let now = now_iso();
    let model = ai_model(env);
    let statement = match db
        .prepare(
            "INSERT INTO schedule_learning (_id, content, ai_model, updated_at) VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(_id) DO UPDATE SET content = excluded.content, ai_model = excluded.ai_model, updated_at = excluded.updated_at",
        )
        .bind(&[content.clone().into(), model.into(), now.clone().into()])
    {
        Ok(statement) => statement,
        Err(_) => return None,
    };
    if statement.run().await.is_err() {
        return None;
    }
    Some(json!({ "content": content, "updated_at": now }))
}

#[worker::send]
pub async fn save_review(
    Extension(env): Extension<SendWrapper<Env>>,
    Path(review_id): Path<i64>,
    Json(payload): Json<ReviewDTO>,
) -> impl IntoResponse {
    if !["completed", "deferred", "skipped"].contains(&payload.outcome.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "outcome must be completed, deferred, or skipped" })),
        )
            .into_response();
    }
    if payload.spent_minutes < 0 || payload.spent_minutes > 1440 || payload.summary.len() > 500 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid spent minutes or summary length" })),
        )
            .into_response();
    }
    let db = match env.d1("DB") {
        Ok(db) => db,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error.to_string() })),
            )
                .into_response()
        }
    };
    let review_statement = match db
        .prepare(
            "SELECT r.task_id, t.title, t.estimated_minutes, t.scheduled_start, t.scheduled_end
             FROM schedule_task_reviews r JOIN schedule_tasks t ON t._id = r.task_id
             WHERE r._id = ?1 AND r.status = 'pending'",
        )
        .bind(&[review_id.into()])
    {
        Ok(statement) => statement,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "review not found or already saved" })),
            )
                .into_response()
        }
    };
    let review = match review_statement.first::<Value>(None).await {
        Ok(Some(row)) => row,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "review not found or already saved" })),
            )
                .into_response()
        }
    };
    let task_id = value_i64(&review, "task_id");
    let estimate = value_i32(&review, "estimated_minutes").clamp(30, MAX_TASK_MINUTES);
    let now = now_iso();
    let next = if payload.outcome == "deferred" {
        let start = match payload
            .next_start
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .and_then(parse_iso)
        {
            Some(start) if start > now_utc() => start,
            _ => {
                let existing = active_task_intervals(&db).await;
                next_available_slot(&env, estimate, None, &existing).start
            }
        };
        Some((start, start + Duration::minutes(estimate as i64)))
    } else {
        None
    };
    if let Err(error) = db
        .prepare(
            "UPDATE schedule_task_reviews
             SET status = ?1, outcome = ?1, spent_minutes = ?2, summary = ?3, reviewed_at = ?4
             WHERE _id = ?5 AND status = 'pending'",
        )
        .bind(&[
            payload.outcome.clone().into(),
            payload.spent_minutes.into(),
            payload.summary.trim().to_string().into(),
            now.clone().into(),
            review_id.into(),
        ])
        .unwrap()
        .run()
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response();
    }
    let task_update = match next {
        Some((start, end)) => db
            .prepare(
                "UPDATE schedule_tasks SET status = 'planned', scheduled_start = ?1, scheduled_end = ?2,
                 actual_minutes = ?3, completion_summary = ?4, updated_at = ?5 WHERE _id = ?6",
            )
            .bind(&[
                format_iso(start).into(),
                format_iso(end).into(),
                payload.spent_minutes.into(),
                payload.summary.trim().to_string().into(),
                now.clone().into(),
                task_id.into(),
            ])
            .unwrap()
            .run()
            .await,
        None if payload.outcome == "completed" => db
            .prepare(
                "UPDATE schedule_tasks SET status = 'done', actual_minutes = ?1, completion_summary = ?2, updated_at = ?3 WHERE _id = ?4",
            )
            .bind(&[
                payload.spent_minutes.into(),
                payload.summary.trim().to_string().into(),
                now.clone().into(),
                task_id.into(),
            ])
            .unwrap()
            .run()
            .await,
        None => db
            .prepare(
                "UPDATE schedule_tasks SET status = 'skipped', actual_minutes = ?1, completion_summary = ?2, updated_at = ?3 WHERE _id = ?4",
            )
            .bind(&[
                payload.spent_minutes.into(),
                payload.summary.trim().to_string().into(),
                now.clone().into(),
                task_id.into(),
            ])
            .unwrap()
            .run()
            .await,
    };
    if let Err(error) = task_update {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response();
    }
    let learning = refresh_learning(&env, &db).await;
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "learning": learning })),
    )
        .into_response()
}

#[worker::send]
pub async fn refresh_learning_endpoint(
    Extension(env): Extension<SendWrapper<Env>>,
) -> impl IntoResponse {
    let db = match env.d1("DB") {
        Ok(db) => db,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error.to_string() })),
            )
                .into_response()
        }
    };
    let learning = refresh_learning(&env, &db).await;
    (StatusCode::OK, Json(json!({ "learning": learning }))).into_response()
}

pub async fn schedule_tick(env: &Env) {
    if let Err(error) = promote_due_tasks(env).await {
        eprintln!("schedule tick failed: {}", error);
    }
}
