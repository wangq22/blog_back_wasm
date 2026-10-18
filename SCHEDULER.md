# Deterministic calendar scheduling

The Worker owns all calendar placement. DeepSeek assesses work parameters and proposes bounded calibration patches; it never supplies schedule timestamps. The pure engine is `src/scheduler.rs`. Orchestration, persistence, reviews and learning live in `src/route/schedule.rs`.

## Deploying an existing calendar

Before deploying this backend, apply **only the new migration** if 0005 and 0006 already exist:

```sh
npx wrangler d1 execute blog --remote --file=./migrations/0009_deterministic_schedule.sql
```

For a new calendar, apply 0005, 0006, then 0009 in order. These ALTER migrations are one-time migrations. They preserve task history and convert existing intervals into individual legacy blocks. Only the existing pending task review links to that legacy block; old reviews and estimates are excluded from numerical calibration. New learning does not interpret the old prose memory as executable policy.

Deploy the backend before the updated frontend. The old task start/end columns remain compatibility summaries; clients must render `blocks[]` to display split tasks correctly. Unscheduled tasks have null summary times. Do not roll back to the old single-block planner after split scheduling is in use without restoring a compatible data snapshot.

No new service is required. `CLOUDFLARE_API_TOKEN`, `SCHEDULE_AI_MODEL`, and the existing AI Gateway configuration are reused. Missing or invalid AI responses use a marked default (60 minutes, difficulty 3, splittable 30–90 minutes); explicit reassessment reuses a previous valid assessment on failure. Each network attempt has a 20-second timeout. Creating fixed events and ordinary replanning require no AI call.

## Policy v2

- Owner timezone: `America/Detroit`, with date-specific IANA offsets; stored timestamps are UTC.
- Planning horizon: 21 local days. Work window: 08:00–22:00.
- Flexible budgets: 240 minutes on weekdays, 360 on weekends. Courses and fixed events occupy additional time.
- Completed flexible work still consumes its local day’s budget: actual minutes when recorded, otherwise planned elapsed minutes; skipped blocks with unknown actual time consume zero. This prevents completing work and immediately allocating another full daily budget.
- Grid and rest: 15 minutes. New work starts at least 15 minutes after the supplied planning clock.
- Started, locked and next-two-hour blocks are protected. Tasks with pending block reviews retain their remaining existing plan until reviewed.
- Fixed events retain exact seconds, can occur outside working hours, and may overlap each other. Conflicts are explicitly reported; flexible work avoids their union. Protected flexible blocks that become invalid after a fixed-event, deadline, availability or budget edit are preserved and flagged as conflicts.
- Dependencies: `depends_on` lists predecessor task IDs. Only tasks whose predecessors have a complete, conflict-free plan are eligible; their earliest start is the latest predecessor block end. All chunks of every predecessor must finish first. Completed tasks satisfy dependencies; skipped/cancelled, unscheduled, partial and pending-review tasks block downstream work. Hard deadlines and fixed successor starts also constrain ancestors. A fixed or protected successor that violates ordering retains its time and reports a conflict; movable downstream blocks are removed when a predecessor becomes invalid. New pending reviews trigger dependency replanning on the next cron tick.
- Priority: `priority_level` uses **P1 highest**, P2 high, P3 normal (default), P4 low, P5 lowest. Legacy numeric `priority` remains 5 highest through 1 lowest, so P1=5 and P2=4. Notes never set priority. Among dependency-ready tasks, urgency comes first: deadlines inside the horizon with available capacity ≤ twice remaining work; then other deadlines inside 72 hours; then other tasks. Within a group: priority descending, deadline ascending, creation time and ID. Capacity is recalculated after each placement.
- Whole remaining work must fit before the hard deadline and inside the horizon. Otherwise no new partial solution is committed; existing protected blocks remain and the task reports a waiting reason. This is sequential task allocation, not a globally optimal solver.

For split tasks, weighted interval dynamic programming finds daily block combinations, then an across-day knapsack selects a complete plan. Unsplit tasks enumerate continuous placements. Integer costs per 15-minute unit are:

```
4 * abs(20 * difficulty - energy)
+ 40 * local_day_index
+ floor(20 * daily_flexible_minutes_after_unit / daily_budget)
+ 15 * moved_unit
```

Add 20 per block. `moved_unit` is 1 for an existing task placed outside its old work cells, otherwise 0. Ties choose earliest completion then lexicographic block start/end times. Inputs include an explicit clock; map iteration, SQL row order and model randomness cannot affect replay of the same persisted snapshot.

Energy buckets (weekday/weekend independently editable): before 10, 10–12, 12–14, 14–16, 16–19, after 19. Defaults: 70, 90, 45, 60, 80, 55.

## Interfaces

Existing task endpoints remain under `/api/protected/schedule/tasks`. Creation/update supports `mode`, `priority_level`, legacy numeric `priority`, `depends_on`, `estimated_minutes`, `difficulty`, `splittable`, `min_chunk_minutes`, `max_chunk_minutes`, `locked`, `fixed_start`, `fixed_end`, and `reassess`. Prefer `priority_level`; conflicting numeric and labeled priorities are rejected. Private task responses return both scales and `depends_on`. `replan` reuses persisted parameters. `use_ai_estimate: true` removes a manual estimate override. Clear a deadline using an empty string. Pure title/note edits do not trigger reassessment.

`depends_on` replaces prerequisites when supplied; omit it to preserve them or send `[]` to clear. The server rejects missing IDs, self-dependencies, duplicate IDs, more than 100 direct predecessors and cycles before committing changes. Deletion returns 409 while any task still references that ID; remove those references first. Dependencies and priority labels use existing task JSON, so this extension requires **no additional database migration** beyond 0009. Old tasks default to no dependencies and retain their numeric priorities.

Times accept offset-qualified RFC3339 or owner-local `YYYY-MM-DDTHH:mm`. Ambiguous/nonexistent local DST times are rejected; use an explicit offset. The legacy task `timezone` label is accepted for compatibility; settings determine local interpretation.

Additional authenticated endpoints:

| Method | Path | Request |
|---|---|---|
| GET | `/schedule/settings` | Read settings and revision |
| PUT | `/schedule/settings` | `{settings: <complete object>, expected_revision}` |
| POST | `/schedule/replan` | Rebuild future flexible work |
| PUT | `/schedule/blocks/{id}` | `{locked: boolean}` |
| GET/POST | `/schedule/learning` | Read profile audit / explicitly retry learning |

Public `/api/schedule/config` returns owner timezone and recurring class slots. Public tasks include actual work blocks, status, deterministic placement explanations and reactions, but exclude notes, assessments and reflections. Completely unscheduled tasks stay private.

MCP exposes the same task/review operations plus `get_schedule_settings`, `update_schedule_settings`, `replan_schedule`, and `lock_schedule_block`; every mutation requires `schedule:write`.

Block review requests send `block_id`, `task_completed`, `remaining_minutes`, optional `spent_minutes`, `energy_rating` (1–5), `defer_reason`, `summary`, and optionally `expected_revision`. Non-completed flexible tasks require a positive current remaining estimate. Actual time is never subtracted as progress. Omitting actual time records it as unknown. Explicit whole-task completion cancels outstanding future blocks and redundant pending reviews. A legacy `outcome=completed` request without `block_id` still completes the entire task.

`next_start` pins the next block and prevents other remaining segments from starting earlier. It checks the work window, 15-minute grid, 21-day horizon, other commitments, prerequisite completion, daily budget and hard deadline. Finish other pending reviews and resolve earlier protected blocks before pinning a later resume time; unlocking the resume block removes its earliest-start constraint. A fixed event cannot be deferred; create a separate follow-up task.

## Learning, concurrency and failures

Every 5-minute tick promotes elapsed blocks to immutable review snapshots; it rebuilds the rolling plan once per local date. Saving a reflection replans with the current profile. Learning is processed separately and applies at the next planning event, avoiding an immediate second reshuffle.

The model proposes category duration factors (500–2000 thousandths) and time-bucket energy targets. The server requires three distinct completed flexible tasks with known actual times for duration calibration, or three explicit same-bucket energy ratings. Each change needs fresh evidence. Legacy and external/interruption outcomes are excluded. Factor changes are capped at 10% per update, energy at 5 points. Sparse history can update prose memory with empty numerical patches. Each processed reflection is consumed once; calibration always references the immutable base estimate, never multiplies an already calibrated estimate. User-entered remaining time is used directly.

Profiles retain full before/after snapshots in `schedule_profile_history`; reviews retain block assessment/configuration versions. Model failure retains the old profile, saves an error and leaves feedback pending for the next cron/manual retry. Stale model output is rejected.

All business changes and calendar results commit in one D1 batch. A revision compare-and-swap establishes a unique random commit token; every subsequent statement is guarded by that token. A stale zero-row CAS therefore makes the entire batch a no-op. Constraint failures roll back the batch. Ordinary planning mutations retry a concurrent conflict once; explicit revision-checked reviews/settings return 409 so user estimates are not silently overwritten. The random commit token is outside the deterministic scheduling input/output.

## Verification

```sh
cargo test --locked
python3 -B -m unittest discover -s tests -p 'test_schedule_storage.py'
worker-build --release
python3 -B tests/schedule_runtime.py --isolated
cargo test --release real_timetable_100_tasks -- --nocapture
```

The isolated runtime test creates a temporary local D1 database and local Worker, exercises real WASM/MCP operations, and cleans them up. It never accesses the production database. The runtime needs localhost socket permissions. Frontend checks: `node --experimental-strip-types tests/schedule.test.mjs`, `npm run check` and `npm run build` in `../blog`; production prerendering requires read access to the existing public blog API.
