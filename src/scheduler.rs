//! Pure, deterministic single-person calendar scheduling. No I/O or implicit clock.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use time::{Date, Duration, OffsetDateTime, PrimitiveDateTime, Time};
use time_tz::{OffsetDateTimeExt, PrimitiveDateTimeExt};

pub const VERSION: &str = "deterministic-v1";
pub const HORIZON: i64 = 21;
pub const QUANTUM: i64 = 900;
pub const GAP: i64 = 900;
pub const CATEGORIES: [&str; 6] = ["coding", "math", "reading", "writing", "admin", "other"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub timezone: String,
    pub work_start: i32,
    pub work_end: i32,
    pub weekday_minutes: i32,
    pub weekend_minutes: i32,
    pub energy_weekday: [i32; 6],
    pub energy_weekend: [i32; 6],
    /// Thousandths; applied to the immutable base estimate, never to a previous estimate.
    pub duration_factors: BTreeMap<String, i32>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            timezone: "America/Detroit".into(),
            work_start: 480,
            work_end: 1320,
            weekday_minutes: 240,
            weekend_minutes: 360,
            energy_weekday: [70, 90, 45, 60, 80, 55],
            energy_weekend: [70, 90, 45, 60, 80, 55],
            duration_factors: BTreeMap::new(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if time_tz::timezones::get_by_name(&self.timezone).is_none() {
            return Err("unknown IANA timezone".into());
        }
        if self.work_start < 0
            || self.work_end > 1440
            || self.work_start >= self.work_end
            || self.work_start % 15 != 0
            || self.work_end % 15 != 0
        {
            return Err("work window must use 15-minute boundaries within a day".into());
        }
        if [self.weekday_minutes, self.weekend_minutes]
            .iter()
            .any(|x| *x < 0 || *x > 840 || *x % 15 != 0)
        {
            return Err("daily budgets must be 0–840 minutes in 15-minute increments".into());
        }
        if self
            .energy_weekday
            .iter()
            .chain(self.energy_weekend.iter())
            .any(|x| !(0..=100).contains(x))
        {
            return Err("energy must be 0–100".into());
        }
        if self
            .duration_factors
            .iter()
            .any(|(k, v)| !CATEGORIES.contains(&k.as_str()) || !(500..=2000).contains(v))
        {
            return Err("invalid category or duration factor".into());
        }
        Ok(())
    }
    pub fn tz(&self) -> &time_tz::Tz {
        time_tz::timezones::get_by_name(&self.timezone).expect("validated timezone")
    }
    pub fn budget(&self, date: Date) -> i32 {
        if weekend(date) {
            self.weekend_minutes
        } else {
            self.weekday_minutes
        }
    }
    pub fn energy(&self, date: Date, minute: i32) -> i32 {
        let values = if weekend(date) {
            self.energy_weekend
        } else {
            self.energy_weekday
        };
        values[bucket(minute)]
    }
}
pub fn weekend(date: Date) -> bool {
    date.weekday().number_days_from_monday() >= 5
}
pub fn bucket(minute: i32) -> usize {
    match minute {
        ..600 => 0,
        600..720 => 1,
        720..840 => 2,
        840..960 => 3,
        960..1140 => 4,
        _ => 5,
    }
}
pub fn local_stamp(settings: &Settings, date: Date, minute: i32) -> Option<i64> {
    (PrimitiveDateTime::new(date, Time::MIDNIGHT) + Duration::minutes(minute as i64))
        .assume_timezone(settings.tz())
        .take()
        .map(|x| x.unix_timestamp())
}
pub fn local_date(settings: &Settings, stamp: i64) -> Date {
    OffsetDateTime::from_unix_timestamp(stamp)
        .expect("timestamp")
        .to_timezone(settings.tz())
        .date()
}
pub fn round_minutes(value: i32) -> i32 {
    ((value.max(1) + 14) / 15) * 15
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Assessment {
    pub estimated_minutes: i32,
    pub difficulty: i32,
    pub category: String,
    pub splittable: bool,
    pub min_chunk_minutes: i32,
    pub max_chunk_minutes: i32,
    pub confidence: i32,
    pub reason: String,
    pub source: String,
    pub model: String,
    pub version: i64,
}
impl Default for Assessment {
    fn default() -> Self {
        Self {
            estimated_minutes: 60,
            difficulty: 3,
            category: "other".into(),
            splittable: true,
            min_chunk_minutes: 30,
            max_chunk_minutes: 90,
            confidence: 0,
            reason: "Default assessment; edit or reassess to refine.".into(),
            source: "fallback".into(),
            model: String::new(),
            version: 1,
        }
    }
}
impl Assessment {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=20160).contains(&self.estimated_minutes)
            || !(1..=5).contains(&self.difficulty)
            || !CATEGORIES.contains(&self.category.as_str())
            || !(0..=100).contains(&self.confidence)
            || self.min_chunk_minutes < 15
            || self.max_chunk_minutes > 840
            || self.min_chunk_minutes > self.max_chunk_minutes
            || self.min_chunk_minutes % 15 != 0
            || self.max_chunk_minutes % 15 != 0
            || self.reason.len() > 1200
        {
            return Err("invalid assessment parameters".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Meta {
    pub mode: String,
    pub priority: i32,
    pub assessment: Assessment,
    pub estimated_override: Option<i32>,
    pub remaining_minutes: Option<i32>,
    pub fixed_start: Option<i64>,
    pub fixed_end: Option<i64>,
    pub locked: bool,
    pub schedule_state: String,
    pub unscheduled_reason: String,
    pub not_before: Option<i64>,
}
impl Default for Meta {
    fn default() -> Self {
        Self {
            mode: "flexible".into(),
            priority: 3,
            assessment: Assessment::default(),
            estimated_override: None,
            remaining_minutes: None,
            fixed_start: None,
            fixed_end: None,
            locked: false,
            schedule_state: "unscheduled".into(),
            unscheduled_reason: String::new(),
            not_before: None,
        }
    }
}
impl Meta {
    pub fn minutes(&self, settings: &Settings) -> i32 {
        if self.mode == "fixed" {
            if let (Some(start), Some(end)) = (self.fixed_start, self.fixed_end) {
                return ((end - start + 59) / 60).max(0) as i32;
            }
        }
        if let Some(r) = self.remaining_minutes {
            return if r == 0 { 0 } else { round_minutes(r) };
        }
        if let Some(r) = self.estimated_override {
            return round_minutes(r);
        }
        round_minutes(
            ((self.assessment.estimated_minutes as i64
                * *settings
                    .duration_factors
                    .get(&self.assessment.category)
                    .unwrap_or(&1000) as i64
                + 999)
                / 1000) as i32,
        )
    }
}
#[derive(Clone, Debug)]
pub struct Task {
    pub id: i64,
    pub created: i64,
    pub deadline: Option<i64>,
    pub status: String,
    pub meta: Meta,
}
impl Task {
    pub fn active(&self) -> bool {
        !["done", "skipped", "cancelled"].contains(&self.status.as_str())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Block {
    pub id: String,
    pub task_id: i64,
    pub start: i64,
    pub end: i64,
    pub minutes: i32,
    pub status: String,
    pub locked: bool,
    pub assessment: Assessment,
    pub settings_version: i64,
    pub legacy: bool,
    #[serde(default)]
    pub actual_minutes: Option<i32>,
}
impl Block {
    pub fn occupies(&self) -> bool {
        ["planned", "in_progress"].contains(&self.status.as_str())
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Busy {
    pub start: i64,
    pub end: i64,
}
impl Busy {
    fn clashes(&self, start: i64, end: i64, gap: i64) -> bool {
        start < self.end + gap && end + gap > self.start
    }
}
#[derive(Clone, Debug)]
pub struct Placement {
    pub task_id: i64,
    pub blocks: Vec<Block>,
    pub state: String,
    pub reason: String,
}
#[derive(Clone)]
struct Candidate {
    start: i64,
    end: i64,
    units: usize,
    cost: i64,
}
#[derive(Clone)]
struct Choice {
    cost: i64,
    spans: Vec<(i64, i64)>,
}
fn better(a: &Choice, b: &Choice) -> bool {
    (a.cost, a.spans.last().map(|x| x.1), &a.spans)
        < (b.cost, b.spans.last().map(|x| x.1), &b.spans)
}
fn retain(slot: &mut Option<Choice>, next: Choice) {
    if slot.as_ref().map(|old| better(&next, old)).unwrap_or(true) {
        *slot = Some(next);
    }
}
fn busy_for_day(settings: &Settings, date: Date, busy: &[Busy]) -> i32 {
    let start = local_stamp(settings, date, 0).unwrap_or(0);
    let end = local_stamp(settings, date + Duration::days(1), 0).unwrap_or(start + 86400);
    busy.iter()
        .map(|b| ((b.end.min(end) - b.start.max(start)).max(0) / 60) as i32)
        .sum()
}
fn daily_options(
    task: &Task,
    settings: &Settings,
    date: Date,
    day: i64,
    now: i64,
    busy: &[Busy],
    allocated: &[Busy],
    old: &[Block],
    need: usize,
) -> Vec<Option<Choice>> {
    let used = busy_for_day(settings, date, allocated);
    let capacity = ((settings.budget(date) - used).max(0) / 15) as usize;
    let limit = capacity.min(need);
    let mut options = vec![None; limit + 1];
    options[0] = Some(Choice {
        cost: 0,
        spans: vec![],
    });
    if limit == 0 {
        return options;
    }
    let min_units = (task.meta.assessment.min_chunk_minutes / 15).min(need as i32) as usize;
    let max_units = (task.meta.assessment.max_chunk_minutes / 15).min(limit as i32) as usize;
    let mut candidates = Vec::new();
    for minute in (settings.work_start..settings.work_end).step_by(15) {
        let Some(start) = local_stamp(settings, date, minute) else {
            continue;
        };
        if start < (now + GAP).max(task.meta.not_before.unwrap_or(i64::MIN)) {
            continue;
        }
        for units in min_units..=max_units {
            let end = start + units as i64 * QUANTUM;
            let Some(day_end) = local_stamp(settings, date, settings.work_end) else {
                continue;
            };
            if end > day_end
                || task.deadline.map(|d| end > d).unwrap_or(false)
                || busy.iter().any(|b| b.clashes(start, end, GAP))
            {
                continue;
            }
            let mut cost = 20;
            for q in 0..units {
                let energy = settings.energy(date, minute + q as i32 * 15);
                let moved = !old.is_empty()
                    && !old.iter().any(|b| {
                        b.start <= start + q as i64 * QUANTUM
                            && b.end >= start + (q + 1) as i64 * QUANTUM
                    });
                cost += 4 * (20 * task.meta.assessment.difficulty - energy).abs() as i64
                    + 40 * day
                    + if moved { 15 } else { 0 };
            }
            candidates.push(Candidate {
                start,
                end,
                units,
                cost,
            });
        }
    }
    candidates.sort_by_key(|c| (c.end, c.start, c.units));
    // Weighted interval DP, additionally indexed by work minutes. Predecessors enforce rest.
    let mut dp = vec![vec![None; limit + 1]; candidates.len() + 1];
    dp[0][0] = options[0].clone();
    for (j, c) in candidates.iter().enumerate() {
        dp[j + 1] = dp[j].clone();
        let predecessor = candidates[..j].partition_point(|p| p.end + GAP <= c.start);
        for units in c.units..=limit {
            if let Some(prev) = dp[predecessor][units - c.units].as_ref() {
                let load: i64 = ((units - c.units + 1)..=units)
                    .map(|q| {
                        20 * (used as i64 + q as i64 * 15) / settings.budget(date).max(1) as i64
                    })
                    .sum();
                let mut next = prev.clone();
                next.cost += c.cost + load;
                next.spans.push((c.start, c.end));
                retain(&mut dp[j + 1][units], next);
            }
        }
    }
    if let Some(last) = dp.pop() {
        options = last;
    }
    options
}
fn fit(
    task: &Task,
    settings: &Settings,
    now: i64,
    busy: &[Busy],
    allocated: &[Busy],
    old: &[Block],
    required: i32,
) -> Option<Choice> {
    if required > available(task, settings, now, busy, allocated) {
        return None;
    }
    let today = local_date(settings, now);
    let need = (required / 15) as usize;
    if need == 0 {
        return Some(Choice {
            cost: 0,
            spans: vec![],
        });
    }
    if !task.meta.assessment.splittable {
        let mut best = None;
        for day in 0..HORIZON {
            let date = today + Duration::days(day);
            if required + busy_for_day(settings, date, allocated) > settings.budget(date) {
                continue;
            }
            for minute in (settings.work_start..settings.work_end).step_by(15) {
                let Some(start) = local_stamp(settings, date, minute) else {
                    continue;
                };
                let end = start + required as i64 * 60;
                if start < (now + GAP).max(task.meta.not_before.unwrap_or(i64::MIN))
                    || end > local_stamp(settings, date, settings.work_end).unwrap_or(start)
                    || task.deadline.map(|d| end > d).unwrap_or(false)
                    || busy.iter().any(|b| b.clashes(start, end, GAP))
                {
                    continue;
                }
                let mut cost = 20;
                for q in 0..need {
                    let moved = !old.is_empty()
                        && !old.iter().any(|b| {
                            b.start <= start + q as i64 * QUANTUM
                                && b.end >= start + (q + 1) as i64 * QUANTUM
                        });
                    cost += 4
                        * (20 * task.meta.assessment.difficulty
                            - settings.energy(date, minute + q as i32 * 15))
                        .abs() as i64
                        + 40 * day
                        + 20 * (busy_for_day(settings, date, allocated) as i64
                            + (q + 1) as i64 * 15)
                            / settings.budget(date).max(1) as i64
                        + if moved { 15 } else { 0 };
                }
                retain(
                    &mut best,
                    Choice {
                        cost,
                        spans: vec![(start, end)],
                    },
                );
            }
        }
        return best;
    }
    // Across-day knapsack: retain the minimum-cost daily packing for each work total.
    let mut dp = vec![None; need + 1];
    dp[0] = Some(Choice {
        cost: 0,
        spans: vec![],
    });
    for day in 0..HORIZON {
        let date = today + Duration::days(day);
        if task
            .deadline
            .map(|d| local_stamp(settings, date, settings.work_start).unwrap_or(i64::MAX) > d)
            .unwrap_or(false)
        {
            break;
        }
        let options = daily_options(task, settings, date, day, now, busy, allocated, old, need);
        let mut next = vec![None; need + 1];
        for total in 0..=need {
            let Some(base) = dp[total].as_ref() else {
                continue;
            };
            for (count, option) in options.iter().enumerate().take(need - total + 1) {
                let Some(option) = option else { continue };
                let mut choice = base.clone();
                choice.cost += option.cost;
                choice.spans.extend_from_slice(&option.spans);
                retain(&mut next[total + count], choice);
            }
        }
        dp = next;
    }
    dp.pop().flatten()
}
fn available(task: &Task, settings: &Settings, now: i64, busy: &[Busy], allocated: &[Busy]) -> i32 {
    let today = local_date(settings, now);
    let mut sum = 0;
    for day in 0..HORIZON {
        let date = today + Duration::days(day);
        let mut minutes = 0;
        for m in (settings.work_start..settings.work_end).step_by(15) {
            if let Some(start) = local_stamp(settings, date, m) {
                let end = start + QUANTUM;
                if start >= (now + GAP).max(task.meta.not_before.unwrap_or(i64::MIN))
                    && !task.deadline.map(|d| end > d).unwrap_or(false)
                    && !busy.iter().any(|b| b.clashes(start, end, GAP))
                {
                    minutes += 15;
                }
            }
        }
        sum +=
            minutes.min((settings.budget(date) - busy_for_day(settings, date, allocated)).max(0));
    }
    sum
}
/// Full rolling rebuild; past blocks are left to the storage layer as immutable history.
pub fn plan(
    settings: &Settings,
    now: i64,
    revision: i64,
    tasks: &[Task],
    blocks: &[Block],
    classes: &[Busy],
) -> Vec<Placement> {
    let mut busy = classes.to_vec();
    let mut allocated = Vec::new();
    // Finished work still consumes today's budget, including tasks now marked done.
    // Actual time is a workload observation, never an estimate of task progress.
    let today = local_date(settings, now);
    let midnight = local_stamp(settings, today, 0).unwrap_or(now);
    for block in blocks
        .iter()
        .filter(|b| b.end <= now && local_date(settings, b.start) == today)
    {
        if !tasks
            .iter()
            .any(|t| t.id == block.task_id && t.meta.mode == "flexible")
        {
            continue;
        }
        let minutes = block
            .actual_minutes
            .unwrap_or(
                if ["skipped", "cancelled"].contains(&block.status.as_str()) {
                    0
                } else {
                    block.minutes
                },
            )
            .clamp(0, settings.budget(today));
        if minutes > 0 {
            allocated.push(Busy {
                start: midnight,
                end: midnight + minutes as i64 * 60,
            });
        }
    }

    let mut result = Vec::new();
    let active: BTreeMap<i64, &Task> = tasks
        .iter()
        .filter(|t| t.active())
        .map(|t| (t.id, t))
        .collect();
    let pending: BTreeSet<i64> = blocks
        .iter()
        .filter(|b| b.status == "awaiting_review" && active.contains_key(&b.task_id))
        .map(|b| b.task_id)
        .collect();
    let mut retained: BTreeMap<i64, Vec<Block>> = BTreeMap::new();
    for b in blocks.iter().filter(|b| b.occupies() && b.end > now) {
        if let Some(task) = active.get(&b.task_id) {
            if task.meta.mode == "fixed" {
                continue;
            }
            if b.locked || task.meta.locked || b.start <= now + 7200 || pending.contains(&b.task_id)
            {
                busy.push(Busy {
                    start: b.start,
                    end: b.end,
                });
                allocated.push(Busy {
                    start: b.start,
                    end: b.end,
                });
                retained.entry(b.task_id).or_default().push(b.clone());
            }
        }
    }
    let mut fixed: Vec<_> = active
        .values()
        .filter(|t| t.meta.mode == "fixed")
        .copied()
        .collect();
    fixed.sort_by_key(|t| t.id);
    let fixed_busy: Vec<_> = fixed
        .iter()
        .filter_map(|t| {
            Some((
                t.id,
                Busy {
                    start: t.meta.fixed_start?,
                    end: t.meta.fixed_end?,
                },
            ))
        })
        .collect();
    for task in fixed {
        let (Some(start), Some(end)) = (task.meta.fixed_start, task.meta.fixed_end) else {
            continue;
        };
        let conflict = busy.iter().any(|b| b.clashes(start, end, 0))
            || fixed_busy
                .iter()
                .any(|(id, b)| *id != task.id && b.clashes(start, end, 0));
        let existing = blocks.iter().find(|b| {
            b.task_id == task.id && b.start == start && b.end == end && b.status != "cancelled"
        });
        let block = existing.cloned().unwrap_or(Block {
            id: format!("{}:{}:fixed", task.id, revision),
            task_id: task.id,
            start,
            end,
            minutes: ((end - start + 59) / 60) as i32,
            status: "planned".into(),
            locked: true,
            assessment: task.meta.assessment.clone(),
            settings_version: revision,
            legacy: false,
            actual_minutes: None,
        });
        busy.push(Busy { start, end });
        result.push(Placement {
            task_id: task.id,
            blocks: vec![block],
            state: if conflict { "conflict" } else { "scheduled" }.into(),
            reason: if conflict {
                "Fixed event overlaps another commitment; choose which to attend."
            } else {
                "Fixed time supplied by owner."
            }
            .into(),
        });
    }
    let mut queue: Vec<_> = active
        .values()
        .filter(|t| t.meta.mode != "fixed")
        .copied()
        .collect();
    while !queue.is_empty() {
        queue.sort_by_cached_key(|task| {
            let r = task.meta.minutes(settings);
            let in_window = task
                .deadline
                .map(|d| d <= now + HORIZON * 86400)
                .unwrap_or(false);
            let urgency = if in_window && available(task, settings, now, &busy, &allocated) <= 2 * r
            {
                0
            } else if task.deadline.map(|d| d <= now + 72 * 3600).unwrap_or(false) {
                1
            } else {
                2
            };
            (
                urgency,
                -task.meta.priority,
                task.deadline.unwrap_or(i64::MAX),
                task.created,
                task.id,
            )
        });
        let task = queue.remove(0);
        let mut kept = retained.remove(&task.id).unwrap_or_default();
        kept.sort_by_key(|b| (b.start, b.end, b.id.clone()));
        if pending.contains(&task.id) {
            result.push(Placement {
                task_id: task.id,
                blocks: kept,
                state: "partial".into(),
                reason: "Review elapsed blocks before changing this task's remaining work.".into(),
            });
            continue;
        }
        let required =
            (task.meta.minutes(settings) - kept.iter().map(|b| b.minutes).sum::<i32>()).max(0);
        let old: Vec<_> = blocks
            .iter()
            .filter(|b| b.task_id == task.id && b.occupies() && b.end > now)
            .cloned()
            .collect();
        let solution = if (!task.meta.assessment.splittable && !kept.is_empty() && required > 0)
            || task.deadline.map(|d| d < now).unwrap_or(false) && required > 0
        {
            None
        } else {
            fit(task, settings, now, &busy, &allocated, &old, required)
        };
        if let Some(solution) = solution {
            for (seq, (start, end)) in solution.spans.into_iter().enumerate() {
                let previous = old.iter().find(|b| b.start == start && b.end == end);
                let block = previous.cloned().unwrap_or(Block {
                    id: format!("{}:{}:{}", task.id, revision, seq),
                    task_id: task.id,
                    start,
                    end,
                    minutes: ((end - start + 59) / 60) as i32,
                    status: "planned".into(),
                    locked: task.meta.locked,
                    assessment: task.meta.assessment.clone(),
                    settings_version: revision,
                    legacy: false,
                    actual_minutes: None,
                });
                busy.push(Busy { start, end });
                allocated.push(Busy { start, end });
                kept.push(block);
            }
            kept.sort_by_key(|b| (b.start, b.end, b.id.clone()));
            result.push(Placement {
                task_id: task.id,
                blocks: kept,
                state: if required == 0 && task.meta.minutes(settings) == 0 {
                    "unscheduled"
                } else {
                    "scheduled"
                }
                .into(),
                reason: if task.meta.minutes(settings) == 0 {
                    "Confirm task completion in its review."
                } else {
                    "Placed by deadline urgency, priority, energy fit and schedule stability."
                }
                .into(),
            });
        } else {
            let reason = if task.deadline.map(|d| d < now).unwrap_or(false) {
                "deadline_passed"
            } else if !task.meta.assessment.splittable {
                "no_contiguous_slot"
            } else {
                "insufficient_capacity_in_21_day_window"
            };
            result.push(Placement {
                task_id: task.id,
                state: if kept.is_empty() {
                    "unscheduled"
                } else {
                    "partial"
                }
                .into(),
                blocks: kept,
                reason: reason.into(),
            });
        }
    }
    // Protected commitments may become invalid after an explicit settings/deadline/fixed-event edit.
    // Preserve their times but never report the resulting calendar as conflict-free.
    let flex_spans: Vec<_> = result
        .iter()
        .filter(|p| {
            active
                .get(&p.task_id)
                .map(|t| t.meta.mode != "fixed")
                .unwrap_or(false)
        })
        .flat_map(|p| {
            p.blocks.iter().map(|b| Busy {
                start: b.start,
                end: b.end,
            })
        })
        .collect();
    for p in &mut result {
        let Some(task) = active.get(&p.task_id) else {
            continue;
        };
        if task.meta.mode == "fixed" {
            continue;
        }
        let invalid = p.blocks.iter().any(|block| {
            let date = local_date(settings, block.start);
            classes
                .iter()
                .chain(fixed_busy.iter().map(|(_, b)| b))
                .any(|busy| busy.clashes(block.start, block.end, 0))
                || task.deadline.map(|d| block.end > d).unwrap_or(false)
                || block.start
                    < local_stamp(settings, date, settings.work_start).unwrap_or(i64::MAX)
                || block.end > local_stamp(settings, date, settings.work_end).unwrap_or(0)
                || busy_for_day(settings, date, &allocated) > settings.budget(date)
                || flex_spans
                    .iter()
                    .filter(|b| b.start == block.start && b.end == block.end)
                    .count()
                    > 1
                || flex_spans.iter().any(|b| {
                    (b.start != block.start || b.end != block.end)
                        && b.clashes(block.start, block.end, 0)
                })
        });
        if invalid {
            p.state = "conflict".into();
            p.reason="Protected blocks conflict with a commitment, deadline, work window or daily budget; resolve them explicitly.".into();
        }
    }
    result.sort_by_key(|p| p.task_id);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    fn stamp(s: &str) -> i64 {
        OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
            .unwrap()
            .unix_timestamp()
    }
    fn task(id: i64, minutes: i32) -> Task {
        let mut meta = Meta::default();
        meta.estimated_override = Some(minutes);
        Task {
            id,
            created: id,
            deadline: None,
            status: "planned".into(),
            meta,
        }
    }
    fn now() -> i64 {
        stamp("2026-10-19T07:00:00-04:00")
    }
    #[test]
    fn reproducible_and_order_independent() {
        let s = Settings::default();
        let a = vec![task(2, 105), task(1, 60)];
        let b = vec![a[1].clone(), a[0].clone()];
        let x = plan(&s, now(), 1, &a, &[], &[]);
        let y = plan(&s, now(), 1, &b, &[], &[]);
        assert_eq!(format!("{:?}", x), format!("{:?}", y));
        assert_eq!(x[1].blocks.iter().map(|b| b.minutes).sum::<i32>(), 105);
    }
    #[test]
    fn no_deadline_violation_or_fallback() {
        let s = Settings::default();
        let mut t = task(1, 240);
        t.deadline = Some(now() + 3600);
        let p = plan(&s, now(), 1, &[t], &[], &[]);
        assert!(p[0].blocks.is_empty());
        assert_eq!(p[0].state, "unscheduled");
    }
    #[test]
    fn splits_respect_budget_gap_and_minimum() {
        let s = Settings::default();
        let p = plan(&s, now(), 1, &[task(1, 600)], &[], &[]);
        let b = &p[0].blocks;
        assert_eq!(b.iter().map(|b| b.minutes).sum::<i32>(), 600);
        for x in b {
            assert!((30..=90).contains(&x.minutes));
        }
        for pair in b.windows(2) {
            assert!(pair[1].start >= pair[0].end + GAP);
        }
        let mut days = BTreeMap::new();
        for x in b {
            *days.entry(local_date(&s, x.start)).or_insert(0) += x.minutes;
        }
        for (d, n) in days {
            assert!(n <= s.budget(d));
        }
    }
    #[test]
    fn high_energy_and_priority() {
        let mut s = Settings::default();
        s.weekday_minutes = 60;
        let mut easy = task(2, 60);
        easy.meta.assessment.difficulty = 1;
        let mut hard = task(1, 60);
        hard.meta.assessment.difficulty = 5;
        hard.meta.priority = 5;
        let p = plan(&s, now(), 1, &[easy, hard], &[], &[]);
        assert_eq!(p[0].blocks[0].start, stamp("2026-10-19T10:00:00-04:00"));
        assert!(p[1].blocks[0].start >= stamp("2026-10-20T00:00:00-04:00"));
    }
    #[test]
    fn fixed_conflicts_never_move() {
        let s = Settings::default();
        let mut t = task(1, 60);
        t.meta.mode = "fixed".into();
        t.meta.fixed_start = Some(now() + 3600);
        t.meta.fixed_end = Some(now() + 7200);
        let busy = [Busy {
            start: now() + 3600,
            end: now() + 7200,
        }];
        let p = plan(&s, now(), 1, &[t, task(2, 60)], &[], &busy);
        assert_eq!(p[0].state, "conflict");
        assert_eq!(p[0].blocks[0].start, now() + 3600);
        assert!(p[1]
            .blocks
            .iter()
            .all(|b| !busy[0].clashes(b.start, b.end, GAP)));
    }
    #[test]
    fn protected_blocks_and_pending_reviews() {
        let s = Settings::default();
        let t = task(1, 60);
        let block = Block {
            id: "old".into(),
            task_id: 1,
            start: now() + 3600,
            end: now() + 7200,
            minutes: 60,
            status: "planned".into(),
            locked: false,
            assessment: Assessment::default(),
            settings_version: 1,
            legacy: false,
            actual_minutes: None,
        };
        let p = plan(&s, now(), 2, &[t.clone()], &[block.clone()], &[]);
        assert_eq!(p[0].blocks[0], block);
        let mut pending = block.clone();
        pending.end = now() - 60;
        pending.start = pending.end - 3600;
        pending.status = "awaiting_review".into();
        let p = plan(&s, now(), 2, &[t], &[pending], &[]);
        assert!(p[0].blocks.is_empty());
        assert_eq!(p[0].state, "partial");
    }
    #[test]
    fn timezone_dst_is_date_specific() {
        let s = Settings::default();
        let a = local_date(&s, stamp("2026-10-31T12:00:00Z"));
        let b = local_date(&s, stamp("2026-11-02T12:00:00Z"));
        assert_eq!(
            local_stamp(&s, a, 600).unwrap(),
            stamp("2026-10-31T14:00:00Z")
        );
        assert_eq!(
            local_stamp(&s, b, 600).unwrap(),
            stamp("2026-11-02T15:00:00Z")
        );
    }
    #[test]
    fn completed_tasks_still_consume_daily_budget_using_actual_time() {
        let mut settings = Settings::default();
        settings.weekday_minutes = 60;
        let clock = stamp("2026-10-19T14:00:00Z");
        let mut done = task(1, 60);
        done.status = "done".into();
        done.meta.remaining_minutes = Some(0);
        let history = Block {
            id: "finished".into(),
            task_id: 1,
            start: stamp("2026-10-19T12:00:00Z"),
            end: stamp("2026-10-19T13:00:00Z"),
            minutes: 60,
            status: "completed".into(),
            locked: false,
            assessment: Assessment::default(),
            settings_version: 0,
            legacy: false,
            actual_minutes: Some(90),
        };
        let tasks = [done, task(2, 30)];
        let p = plan(&settings, clock, 1, &tasks, &[history.clone()], &[]);
        assert!(p[0].blocks[0].start >= stamp("2026-10-20T00:00:00-04:00"));
        let mut event = task(3, 60);
        event.meta.mode = "fixed".into();
        event.meta.fixed_start = Some(stamp("2026-10-19T00:30:00-04:00"));
        event.meta.fixed_end = Some(stamp("2026-10-19T01:30:00-04:00"));
        let with_event = [tasks[0].clone(), tasks[1].clone(), event];
        let p = plan(&settings, clock, 1, &with_event, &[history.clone()], &[]);
        assert_eq!(
            p.iter().find(|p| p.task_id == 3).unwrap().state,
            "scheduled"
        );
        let mut zero = history;
        zero.actual_minutes = Some(0);
        let p = plan(&settings, clock, 1, &tasks, &[zero], &[]);
        assert_eq!(
            local_date(&settings, p[0].blocks[0].start),
            local_date(&settings, clock)
        );
    }
    #[test]
    fn pinned_resume_prevents_earlier_remaining_segments() {
        let settings = Settings::default();
        let release = stamp("2026-10-20T17:00:00-04:00");
        let mut t = task(1, 120);
        t.meta.not_before = Some(release);
        let anchor = Block {
            id: "manual".into(),
            task_id: 1,
            start: release,
            end: release + 90 * 60,
            minutes: 90,
            status: "planned".into(),
            locked: true,
            assessment: Assessment::default(),
            settings_version: 0,
            legacy: false,
            actual_minutes: None,
        };
        let p = plan(&settings, now(), 1, &[t], &[anchor], &[]);
        assert_eq!(p[0].blocks.iter().map(|b| b.minutes).sum::<i32>(), 120);
        assert!(p[0].blocks.iter().all(|b| b.start >= release));
    }
    #[test]
    fn benchmark_100_tasks() {
        let s = Settings::default();
        let tasks: Vec<_> = (1..=100)
            .map(|i| task(i, 30 + (i as i32 % 4) * 15))
            .collect();
        let start = Instant::now();
        let p = plan(&s, now(), 1, &tasks, &[], &[]);
        eprintln!("100 tasks / 21 days: {:?}", start.elapsed());
        assert_eq!(p.len(), 100);
        let mut spans: Vec<_> = p
            .iter()
            .flat_map(|p| p.blocks.iter().map(|b| (b.start, b.end)))
            .collect();
        spans.sort();
        for pair in spans.windows(2) {
            assert!(pair[0].1 + GAP <= pair[1].0);
        }
    }
}
