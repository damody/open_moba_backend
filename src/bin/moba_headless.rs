//! Single-lane acceptance executable: real script DLL, formal inputs, replay.
//! Execution budgets and the objective stall monitor are headless diagnostics, not game rules.
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use failure::{err_msg, Error};
use omoba_core::runtime::*;
use serde_json::{json, Value};
use specs::Join;
use specs::WorldExt;
use omoba_core::runtime::native::moba_match::bots::{role_bot_inputs, RoleBotConfig, RoleBotMatchPlan};

const DEFAULT_MAX_GAME_SECONDS: u64 = 600;
const DEFAULT_STALL_GAME_SECONDS: u64 = 300;
const MIN_BUDGET_SECONDS: u64 = 60;
const MAX_BUDGET_SECONDS: u64 = 3600;
const DIAGNOSTIC_SAMPLE_LIMIT: usize = 10;
const DIAGNOSTIC_SAMPLE_INTERVAL_SECONDS: u64 = 60;
const MAX_OBJECTIVE_SLOTS: usize = 64;
const BUDGET_SCOPE: &str = "headless execution budget only; not a game rule or win condition";
const STALL_SCOPE: &str = "continuous absence of public tower or base real HP decrease, retirement, or layer replacement; waves, hero movement, kills, and camp respawns are not objective progress";
const PLAN_ONLY_SCOPE: &str = "configuration only; no simulation or match acceptance";
const FAILURE_SCOPE: &str = "bounded failure diagnostic; not a match result or acceptance";

/// One formal tick: the driver step and, with role bots, the committed visibility wave that
/// follows it. The run and its replay both step through here, so a replay never skips work
/// the run did (E161: the replay skipped the wave and diverged at tick 362).
fn step_tick(
    driver: &mut SimulationDriver,
    world: &mut specs::World,
    inputs: Vec<(u32, PlayerInput)>,
    with_bots: bool,
) -> Result<SimulationTickResult, Error> {
    let started = std::time::Instant::now();
    let result = driver.step(world, inputs)?;
    if with_bots {
        let wave = std::time::Instant::now();
        run_committed_visibility_wave_b(world, result.tick, 0);
        VISIBILITY_WALL_NANOS.fetch_add(wave.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
    }
    // Wall time of the formal ticks (E187 §5.6 / §5.8: per-tick cost of a map), run and replay.
    STEP_WALL_NANOS.fetch_add(started.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
    STEP_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(result)
}

static STEP_WALL_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static STEP_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The bots' committed visibility wave part of [`STEP_WALL_NANOS`].
static VISIBILITY_WALL_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn visibility_wall_ms_mean() -> f64 {
    let count = STEP_COUNT.load(std::sync::atomic::Ordering::Relaxed).max(1);
    VISIBILITY_WALL_NANOS.load(std::sync::atomic::Ordering::Relaxed) as f64 / count as f64 / 1e6
}

/// Mean wall milliseconds per formal tick (driver step plus the bots' visibility wave) so far.
fn step_wall_ms_mean() -> f64 {
    let count = STEP_COUNT.load(std::sync::atomic::Ordering::Relaxed).max(1);
    STEP_WALL_NANOS.load(std::sync::atomic::Ordering::Relaxed) as f64 / count as f64 / 1e6
}

fn make_world(config: SingleLaneConfig, scripts_dir: &Path) -> Result<specs::World, Error> {
    let scripts = omoba_core::scripting::loader::load_scripts_dir(scripts_dir);
    create_single_lane_world(config, scripts)
}

struct HeadlessArgs {
    scripts_dir: PathBuf,
    report_path: PathBuf,
    config: SingleLaneConfig,
    profile: SimulationTickProfile,
    withdraw_defender: bool,
    role_plan_path: Option<PathBuf>,
    plan_only: bool,
    max_game_seconds: u64,
    stall_game_seconds: u64,
    /// M3' ② acceptance: before stepping tick index TICK (run and replay alike), swap a script
    /// DLL as a dev hot reload does (`--hot-swap TICK=MODULE=PATH`).
    hot_swap: Option<(usize, omoba_core::game_proto::ScriptModuleSwap)>,
}

/// `TICK=MODULE=PATH`: the swap omoba-core's dev hot reload would apply before that tick.
fn parse_hot_swap(value: &str) -> Result<(usize, omoba_core::game_proto::ScriptModuleSwap), Error> {
    let mut parts = value.splitn(3, '=');
    let (Some(tick), Some(module), Some(file)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(err_msg("--hot-swap requires TICK=MODULE=PATH"));
    };
    let tick: usize = tick.parse().map_err(|_| err_msg("--hot-swap TICK must be a tick index"))?;
    let sha256 = omoba_core::scripting::loader::file_sha256(std::path::Path::new(file)).map_err(err_msg)?;
    Ok((tick, omoba_core::game_proto::ScriptModuleSwap { module: module.into(), file: file.into(), sha256, generation: 1 }))
}

fn apply_hot_swap_at(
    index: usize,
    hot_swap: &Option<(usize, omoba_core::game_proto::ScriptModuleSwap)>,
    driver: &mut SimulationDriver,
    world: &mut specs::World,
) -> Result<(), Error> {
    if let Some((tick, swap)) = hot_swap {
        if *tick == index {
            omoba_core::scripting::loader::apply_hot_swap(driver.scripts_mut(), world, swap).map_err(err_msg)?;
        }
    }
    Ok(())
}

fn parse_budget_seconds(value: &str, name: &str) -> Result<u64, Error> {
    let digits = !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    let canonical = digits && !value.starts_with('0');
    if !canonical {
        return Err(err_msg(format!(
            "{name} must be an integer from {MIN_BUDGET_SECONDS} to {MAX_BUDGET_SECONDS}"
        )));
    }
    let parsed: u64 = match value.parse() {
        Ok(parsed) => parsed,
        Err(_) => return Err(err_msg(format!("{name} overflow"))),
    };
    if !(MIN_BUDGET_SECONDS..=MAX_BUDGET_SECONDS).contains(&parsed) {
        return Err(err_msg(format!(
            "{name} must be an integer from {MIN_BUDGET_SECONDS} to {MAX_BUDGET_SECONDS}"
        )));
    }
    Ok(parsed)
}

fn checked_game_ticks(seconds: u64, ticks_per_second: u32) -> Result<u64, Error> {
    u64::from(ticks_per_second).checked_mul(seconds).ok_or_else(|| {
        err_msg(format!("tick budget overflow: {ticks_per_second}Hz * {seconds}s"))
    })
}

fn parse_headless_args<I, S>(args: I) -> Result<HeadlessArgs, Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut scripts_dir = PathBuf::from("scripts/target/release");
    let mut report_path = PathBuf::from("omb/target/moba-headless/report.json");
    let mut config = SingleLaneConfig::default();
    let mut profile = SimulationTickProfile::Production60Hz;
    let mut withdraw_defender = false;
    let mut role_plan_path = None;
    let mut plan_only = false;
    let mut fixture_options = false;
    let mut max_game_seconds = DEFAULT_MAX_GAME_SECONDS;
    let mut stall_game_seconds = DEFAULT_STALL_GAME_SECONDS;
    let mut seen_report = false;
    let mut seen_role = false;
    let mut seen_max = false;
    let mut seen_stall = false;
    let mut hot_swap = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let arg = arg.as_ref();
        let value = args
            .next()
            .ok_or_else(|| err_msg(format!("{arg} requires a value")))?;
        let value = value.as_ref();
        match arg {
            "--scripts-dir" => scripts_dir = PathBuf::from(value),
            "--hot-swap" => hot_swap = Some(parse_hot_swap(value)?),
            "--report" => {
                if seen_report {
                    return Err(err_msg("duplicate report option"));
                }
                seen_report = true;
                report_path = PathBuf::from(value);
            }
            "--role-plan" => {
                if seen_role {
                    return Err(err_msg("duplicate role-plan option"));
                }
                seen_role = true;
                role_plan_path = Some(PathBuf::from(value));
            }
            "--plan-only" => {
                plan_only = value
                    .parse()
                    .map_err(|_| err_msg("plan-only must be true or false"))?
            }
            "--seed" => config.seed = value.parse().map_err(|_| err_msg("invalid seed"))?,
            "--map" => {
                fixture_options = true;
                let map = omoba_template_ids::moba_map_by_name(value)
                    .ok_or_else(|| err_msg(format!("unknown compiled MOBA map: {value}")))?;
                config.lane_length = omoba_sim::Fixed64::from_i32(map.lane_length);
                config.tower_hp = omoba_sim::Fixed64::from_i32(map.tower_hp);
                config.base_hp = omoba_sim::Fixed64::from_i32(map.base_hp);
                config.hero_xp_scale_permille = map.hero_xp_scale_permille();
                config.map_id = Some(value.to_string());
            }
            "--defender" => {
                fixture_options = true;
                withdraw_defender = match value {
                    "guard" => false,
                    "withdraw" => true,
                    _ => return Err(err_msg("defender must be guard or withdraw")),
                };
            }
            "--profile" => {
                profile = match value {
                    "15" => SimulationTickProfile::Coarse15Hz,
                    "60" => SimulationTickProfile::Production60Hz,
                    "120" => SimulationTickProfile::Production120Hz,
                    _ => return Err(err_msg("profile must be 15, 60 or 120")),
                }
            }
            "--max-game-seconds" => {
                if seen_max {
                    return Err(err_msg("duplicate max-game-seconds option"));
                }
                seen_max = true;
                max_game_seconds = parse_budget_seconds(value, "max-game-seconds")?;
            }
            "--stall-game-seconds" => {
                if seen_stall {
                    return Err(err_msg("duplicate stall-game-seconds option"));
                }
                seen_stall = true;
                stall_game_seconds = parse_budget_seconds(value, "stall-game-seconds")?;
            }
            _ => return Err(err_msg(format!("unknown argument: {arg}"))),
        }
    }
    if stall_game_seconds > max_game_seconds {
        return Err(err_msg(format!(
            "stall-game-seconds {stall_game_seconds} exceeds max-game-seconds {max_game_seconds}"
        )));
    }
    if role_plan_path.is_some() && fixture_options {
        return Err(err_msg("role-plan cannot be combined with fixture map/defender options"));
    }
    Ok(HeadlessArgs {
        scripts_dir,
        report_path,
        config,
        profile,
        withdraw_defender,
        role_plan_path,
        plan_only,
        max_game_seconds,
        stall_game_seconds,
        hot_swap,
    })
}

fn run() -> Result<(), Error> {
    let parsed = parse_headless_args(std::env::args().skip(1))?;
    // Refuse before simulation so a long run cannot replace an existing report or diagnostic.
    reject_existing_outputs(&parsed.report_path)?;
    let profile = parsed.profile;
    let hz = profile.ticks_per_game_second();
    let max_ticks = checked_game_ticks(parsed.max_game_seconds, hz)?;
    let stall_ticks = checked_game_ticks(parsed.stall_game_seconds, hz)?;
    let mut config = parsed.config;
    let role_plan = parsed.role_plan_path.as_ref().map(|path| -> Result<RoleBotMatchPlan, Error> {
        let mut plan: RoleBotMatchPlan = serde_json::from_slice(&std::fs::read(path)?)?;
        plan.resolve_bot_hero_conflicts();
        Ok(plan)
    }).transpose()?;
    let bots = if let Some(plan) = &role_plan {
        let (planned, bots) = plan.compile(config.seed, profile).map_err(err_msg)?;
        config = planned;
        Some(bots)
    } else {
        None
    };
    if parsed.plan_only {
        let plan = role_plan.as_ref().ok_or_else(|| err_msg("plan-only requires role-plan"))?;
        let bots = bots.as_ref().expect("compiled plan");
        let report = json!({
            "success": true,
            "match_played": false,
            "scope": PLAN_ONLY_SCOPE,
            "profile_hz": hz,
            "max_game_seconds": parsed.max_game_seconds,
            "stall_game_seconds": parsed.stall_game_seconds,
            "max_ticks": max_ticks,
            "stall_ticks": stall_ticks,
            "budget_scope": BUDGET_SCOPE,
            "stall_scope": STALL_SCOPE,
            "plan": plan,
            "bot_player_ids": bots.assignments.iter().map(|assignment| assignment.player_id).collect::<Vec<_>>(),
            "think_interval_ticks": bots.think_interval_ticks,
        });
        write_new_output(&parsed.report_path, &serde_json::to_vec_pretty(&report)?)?;
        println!(
            "MOBA role plan valid: {} players, {} bots",
            plan.players.len(),
            bots.assignments.len()
        );
        return Ok(());
    }
    let scripts_dir = parsed.scripts_dir;
    let report_path = parsed.report_path;
    let withdraw_defender = parsed.withdraw_defender;
    let max_game_seconds = parsed.max_game_seconds;
    let stall_game_seconds = parsed.stall_game_seconds;
    let hot_swap = parsed.hot_swap;
    let mut world = make_world(config.clone(), &scripts_dir)?;
    if let Some(bots) = &bots {
        bots.validate(&world.read_resource::<MobaMatch>()).map_err(err_msg)?;
    }
    let mut driver = SimulationDriver::from_world(&mut world, profile)?;
    let mut inputs_recorded = Vec::new();
    let mut digests = Vec::new();
    let mut casts = [0u32; 4];
    // Per-hero, per-ability statistics for batch acceptance (E265): bot cast inputs by slot,
    // accepted casts (Ability facts), the cast trace's results, summons and K/D/A.
    let mut hero_stats = std::env::var("OMB_HEADLESS_HERO_STATS").is_ok_and(|v| v == "1").then(HeroStats::default);
    let mut tick_perf = TickPerf::new(hz);
    if hero_stats.is_some() { insert_stat_traces(&mut world); }
    let mut end_events = 0usize;
    let mut combat_facts = 0usize;
    // Objective slots are a bounded public read. Full snapshots stay on the sample interval.
    let mut monitor = ObjectiveMonitor::baseline(collect_objective_slots(&world));
    let mut samples = Vec::new();
    let mut last_tick = 0u64;
    for index in 0..max_ticks as usize {
        apply_hot_swap_at(index, &hot_swap, &mut driver, &mut world)?;
        let mut inputs = if let Some(bots) = &bots {
            role_bot_inputs(&world, bots).map_err(err_msg)?
        } else {
            single_lane_bot_inputs(&world, [SingleLaneBotPolicy::Push, SingleLaneBotPolicy::Guard])
        };
        if withdraw_defender && world.read_resource::<MobaMatch>().phase == MobaMatchPhase::Playing {
            // A noncompetitive completion fixture using only ordinary inputs.
            // No health, tower, progression or phase injection.
            inputs.retain(|(player, _)| *player != config.players[1]);
            inputs.push((
                config.players[1],
                PlayerInput {
                    action: Some(PlayerInputEnum::MoveTo(MoveTo {
                        target: Some(Vec2I {
                            x: config.lane_length.raw() as i32,
                            y: 3000 * 1024,
                        }),
                        queued: false,
                    })),
                },
            ));
        }
        for (_, input) in &inputs {
            if let Some(PlayerInputEnum::CastAbility(cast)) = &input.action {
                if let Some(count) = casts.get_mut(cast.ability_index as usize) {
                    *count += 1;
                }
            }
        }
        let step_started = std::time::Instant::now();
        let result = step_tick(&mut driver, &mut world, inputs.clone(), bots.is_some())?;
        tick_perf.record(&world, result.tick, step_started.elapsed().as_nanos() as u64);
        if let Some(stats) = &mut hero_stats { stats.observe(&world, &inputs, &result.facts); }
        combat_facts += result
            .facts
            .iter()
            .filter(|fact| matches!(fact.fact, ObservableFact::DirectCombat { .. }))
            .count();
        end_events += result.events.iter().filter(|event| event.topic == "game.end").count();
        inputs_recorded.push(inputs);
        digests.push(single_lane_replay_digest(&world));
        let tick = result.tick;
        last_tick = tick;
        let stalled = monitor.observe(tick, collect_objective_slots(&world), stall_ticks);
        let finished = matches!(world.read_resource::<MobaMatch>().phase, MobaMatchPhase::Finished { .. });
        if !finished && diagnostic_sample_due(tick, hz) {
            remember_diagnostic_sample(
                &mut samples,
                bounded_match_snapshot(&world, bots.as_ref(), tick),
            );
        }
        if finished {
            break;
        }
        if stalled.is_some() {
            if samples.last().and_then(|sample| sample.get("tick")).and_then(Value::as_u64) != Some(tick) {
                remember_diagnostic_sample(
                    &mut samples,
                    bounded_match_snapshot(&world, bots.as_ref(), tick),
                );
            }
            let waves = world.read_resource::<MobaMatch>().waves;
            let message = stall_message(stall_game_seconds, monitor.last_progress_tick, tick);
            return write_failure(
                &report_path,
                &failure_document(
                    "objective_stall",
                    &message,
                    hz,
                    config.seed,
                    &config.map_id,
                    waves,
                    max_game_seconds,
                    stall_game_seconds,
                    max_ticks,
                    stall_ticks,
                    monitor.last_progress_tick,
                    tick,
                    &samples,
                ),
            );
        }
    }
    if let Some((tick, swap)) = &hot_swap {
        // M3' ② acceptance: the run's per-tick digests, verified by a replay that swaps at the same
        // tick, written whether or not the match finished inside the budget.
        let mut replay = make_world(config.clone(), &scripts_dir)?;
        if hero_stats.is_some() { insert_stat_traces(&mut replay); }
        let mut replay_driver = SimulationDriver::from_world(&mut replay, profile)?;
        for (index, (inputs, expected)) in inputs_recorded.iter().zip(&digests).enumerate() {
            if hero_stats.is_some() { clear_stat_traces(&replay); }
            apply_hot_swap_at(index, &hot_swap, &mut replay_driver, &mut replay)?;
            step_tick(&mut replay_driver, &mut replay, inputs.clone(), bots.is_some())?;
            let actual = single_lane_replay_digest(&replay);
            if &actual != expected {
                return Err(err_msg(format!("hot-swap replay diverged at tick {}: {expected} != {actual}", index + 1)));
            }
        }
        let path = report_path.with_extension("digests.json");
        write_new_output(&path, &serde_json::to_vec_pretty(&json!({
            "hot_swap": {"tick": tick, "module": swap.module, "file": swap.file, "sha256": swap.sha256},
            "script_modules": omoba_core::scripting::loader::script_modules().iter()
                .map(|m| json!({"module": m.module, "sha256": m.sha256})).collect::<Vec<_>>(),
            "replay_verified_ticks": digests.len(),
            "tick_digests": digests,
        }))?)?;
        println!("hot-swap digests ({} ticks, replay verified): {}", digests.len(), path.display());
    }
    let state = world.read_resource::<MobaMatch>().clone();
    let (winner_side, winner_team, finish_tick) = match state.phase {
        MobaMatchPhase::Finished { winner, tick } => {
            let (winner_side, winner_team) = reported_match_winner(&state.config.teams, winner)?;
            (winner_side, winner_team, tick)
        }
        phase => {
            if samples.last().and_then(|sample| sample.get("tick")).and_then(Value::as_u64) != Some(last_tick) {
                remember_diagnostic_sample(
                    &mut samples,
                    bounded_match_snapshot(&world, bots.as_ref(), last_tick),
                );
            }
            let message = timeout_message(
                max_game_seconds,
                max_ticks,
                monitor.last_progress_tick,
                &format!("{phase:?}"),
            );
            return write_failure(
                &report_path,
                &failure_document(
                    "execution_budget",
                    &message,
                    hz,
                    config.seed,
                    &config.map_id,
                    state.waves,
                    max_game_seconds,
                    stall_game_seconds,
                    max_ticks,
                    stall_ticks,
                    monitor.last_progress_tick,
                    last_tick,
                    &samples,
                ),
            );
        }
    };
    if end_events != 1 || (bots.is_none() && casts.contains(&0)) || combat_facts == 0 {
        return Err(err_msg(format!(
            "incomplete acceptance: end_events={end_events}, casts={casts:?}"
        )));
    }
    let mut replay = make_world(config.clone(), &scripts_dir)?;
    if hero_stats.is_some() { insert_stat_traces(&mut replay); }
    let mut replay_driver = SimulationDriver::from_world(&mut replay, profile)?;
    for (index, (inputs, expected)) in inputs_recorded.into_iter().zip(&digests).enumerate() {
        if hero_stats.is_some() { clear_stat_traces(&replay); }
        step_tick(&mut replay_driver, &mut replay, inputs, bots.is_some())?;
        let actual = single_lane_replay_digest(&replay);
        if &actual != expected {
            return Err(err_msg(format!(
                "replay diverged at tick {}: {expected} != {actual}",
                index + 1
            )));
        }
    }
    // Lane towers still standing per side over every layer (E187: both sides' pushes, not only the winner's).
    // Paired towers (nobu, E187 §5.2) count both towers of a layer.
    let towers_standing = [0usize, 1].map(|side| state.lane_tower_layers.iter().chain(&state.lane_tower_partners).flatten()
        .filter(|layer| layer[side].is_some_and(|tower| world.entities().is_alive(tower))).count());
    let towers_total = state.lane_tower_layers.iter().map(Vec::len).sum::<usize>()
        * if state.lane_tower_partners.is_empty() { 1 } else { 2 };
    let report = json!({
        "success": true,
        "match_played": true,
        "profile_hz": hz,
        "seed": config.seed,
        "role_plan": role_plan,
        "bot_mode": if bots.is_some() { "committed_role_plan" } else { "legacy_fixture" },
        "skill_coverage_checked": bots.is_none(),
        "map_id": config.map_id,
        "lane_count": state.lane_towers.len(),
        "defender_policy": if withdraw_defender { "withdraw" } else { "guard" },
        "remaining_lane_towers": state.lane_towers.iter().map(|lane|
            lane.map(|tower| tower.is_some())).collect::<Vec<_>>(),
        "lane_towers_standing_by_side": towers_standing,
        "lane_towers_total_per_side": towers_total,
        "step_wall_ms_mean": step_wall_ms_mean(),
        "visibility_wave_ms_mean": visibility_wall_ms_mean(),
        "step_count": STEP_COUNT.load(std::sync::atomic::Ordering::Relaxed),
        "performance": tick_perf.report(),
        "script_dll": scripts_dir.join("base_content.dll"),
        "winner_side": winner_side,
        "winner_team": winner_team,
        "finish_tick": finish_tick,
        "game_seconds": state.elapsed.raw() as f64 / 1024.0,
        "waves": state.waves,
        "deaths": state.heroes.iter().map(|hero| hero.deaths).collect::<Vec<_>>(),
        "respawns": state.heroes.iter().map(|hero| hero.respawns).collect::<Vec<_>>(),
        "cast_inputs_by_slot": casts,
        "hero_stats": hero_stats.as_ref().map(|stats| stats.report(&world)),
        "creep_facing": hero_stats.as_ref().map(|stats| stats.creep_facing.report()),
        "end_events": end_events,
        "committed_combat_facts": combat_facts,
        "replay_verified_ticks": digests.len(),
        "final_digest": digests.last(),
        "tick_digests": digests,
        "max_game_seconds": max_game_seconds,
        "stall_game_seconds": stall_game_seconds,
        "max_ticks": max_ticks,
        "stall_ticks": stall_ticks,
        "last_progress_tick": monitor.last_progress_tick,
        "budget_scope": BUDGET_SCOPE,
        "stall_scope": STALL_SCOPE,
        "scope": "authoritative headless ECS; not Unreal, filtered replica, LAN, or full game acceptance"
    });
    write_new_output(&report_path, &serde_json::to_vec_pretty(&report)?)?;
    println!(
        "MOBA headless PASS: winner_side={winner_side:?}, winner_team={winner_team:?}, tick={finish_tick}, waves={}, casts={casts:?}, replay={} ticks; report={}",
        state.waves,
        report["replay_verified_ticks"],
        report_path.display()
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum ObjectiveKind {
    Base = 1,
    TowerFront = 2,
    TowerLayer = 3,
    /// Soldier camps (inhibitors) and general towers (E187): a push grinding the base structures is progress.
    BaseStructure = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObjectiveSlot {
    kind: ObjectiveKind,
    lane: u16,
    layer: u16,
    team_id: u32,
    entity_id: u32,
    entity_gen: i32,
    hp_raw: i64,
}

struct ObjectiveMonitor {
    previous: Vec<ObjectiveSlot>,
    last_progress_tick: u64,
}

impl ObjectiveMonitor {
    fn baseline(slots: Vec<ObjectiveSlot>) -> Self {
        Self {
            previous: bound_objective_slots(slots),
            last_progress_tick: 0,
        }
    }

    /// Returns the previous progress tick when this tick is a stall. Progress resets the clock.
    fn observe(&mut self, tick: u64, slots: Vec<ObjectiveSlot>, stall_ticks: u64) -> Option<u64> {
        let slots = bound_objective_slots(slots);
        let progressed = objective_slots_progressed(&self.previous, &slots);
        self.previous = slots;
        if progressed {
            self.last_progress_tick = tick;
            return None;
        }
        if stall_ticks > 0 && tick.saturating_sub(self.last_progress_tick) >= stall_ticks {
            return Some(self.last_progress_tick);
        }
        None
    }
}

fn bound_objective_slots(mut slots: Vec<ObjectiveSlot>) -> Vec<ObjectiveSlot> {
    slots.sort_by_key(|slot| (slot.kind as u8, slot.lane, slot.layer, slot.team_id));
    slots.truncate(MAX_OBJECTIVE_SLOTS);
    slots
}

fn objective_key(slot: &ObjectiveSlot) -> (u8, u16, u16, u32) {
    (slot.kind as u8, slot.lane, slot.layer, slot.team_id)
}

fn objective_slot_present(slot: &ObjectiveSlot) -> bool {
    slot.entity_gen != 0
}

/// Real public structure change since the previous observation. Not a game rule.
fn objective_slots_progressed(before: &[ObjectiveSlot], after: &[ObjectiveSlot]) -> bool {
    let left = bound_objective_slots(before.to_vec());
    let right = bound_objective_slots(after.to_vec());
    let mut next = BTreeMap::new();
    for slot in &right {
        next.insert(objective_key(slot), *slot);
    }
    for slot in &left {
        match next.get(&objective_key(slot)) {
            None => return true,
            Some(updated) => {
                // Generation 0 is the absent slot, including the dead tuple (0, 0, 0).
                // Entity id 0 is a legal living id when its generation is non-zero.
                let was_alive = objective_slot_present(slot);
                let is_alive = objective_slot_present(updated);
                if was_alive && !is_alive {
                    return true;
                }
                if !was_alive && is_alive {
                    return true;
                }
                if was_alive
                    && is_alive
                    && (slot.entity_id != updated.entity_id || slot.entity_gen != updated.entity_gen)
                {
                    return true;
                }
                if was_alive
                    && is_alive
                    && slot.entity_id == updated.entity_id
                    && slot.entity_gen == updated.entity_gen
                    && updated.hp_raw < slot.hp_raw
                {
                    return true;
                }
            }
        }
    }
    false
}

struct HeadlessObservation {
    objectives: Vec<ObjectiveSlot>,
    waves: u32,
    hero_moves: u32,
    hero_kills: u32,
    camp_respawns: u32,
    private_commands: u32,
}

fn objective_progressed(before: &HeadlessObservation, after: &HeadlessObservation) -> bool {
    objective_slots_progressed(&before.objectives, &after.objectives)
}

fn collect_objective_slots(world: &specs::World) -> Vec<ObjectiveSlot> {
    let state = world.read_resource::<MobaMatch>();
    let properties = world.read_storage::<CProperty>();
    let entities = world.entities();
    let mut slots = Vec::new();
    let mut push = |kind: ObjectiveKind, lane: usize, layer: usize, team_id: u32, entity: Option<specs::Entity>| {
        if slots.len() >= MAX_OBJECTIVE_SLOTS {
            return;
        }
        let (entity_id, entity_gen, hp_raw) = match entity.filter(|entity| entities.is_alive(*entity)) {
            Some(entity) => (
                entity.id(),
                entity.gen().id(),
                properties.get(entity).map(|prop| prop.hp.raw()).unwrap_or(0),
            ),
            None => (0, 0, 0),
        };
        slots.push(ObjectiveSlot {
            kind,
            lane: u16::try_from(lane).unwrap_or(u16::MAX),
            layer: u16::try_from(layer).unwrap_or(u16::MAX),
            team_id,
            entity_id,
            entity_gen,
            hp_raw,
        });
    };
    for (side, base) in state.bases.iter().enumerate() {
        push(ObjectiveKind::Base, 0, 0, state.config.teams[side], *base);
    }
    for (lane, sides) in state.lane_towers.iter().enumerate() {
        for (side, tower) in sides.iter().enumerate() {
            push(ObjectiveKind::TowerFront, lane, 0, state.config.teams[side], *tower);
        }
    }
    for (lane, layers) in state.lane_tower_layers.iter().enumerate() {
        for (layer, sides) in layers.iter().enumerate() {
            for (side, tower) in sides.iter().enumerate() {
                push(ObjectiveKind::TowerLayer, lane, layer, state.config.teams[side], *tower);
            }
        }
    }
    for (team, lane, role, entity, _, _) in state.base_structure_rows() {
        let layer = if role == LaneRole::Inhibitor { 0 } else { 1 };
        push(ObjectiveKind::BaseStructure, lane, layer, state.config.teams[team], entity);
    }
    bound_objective_slots(slots)
}

fn stall_message(stall_game_seconds: u64, last_progress_tick: u64, tick: u64) -> String {
    format!(
        "no public tower/base objective progress for {stall_game_seconds} game seconds; last_progress_tick={last_progress_tick}; tick={tick}; waves, hero movement, kills, and camp respawns do not count; stall budget is not a game rule"
    )
}

fn timeout_message(max_game_seconds: u64, max_ticks: u64, last_progress_tick: u64, phase: &str) -> String {
    format!(
        "execution budget exhausted after {max_game_seconds} game seconds ({max_ticks} ticks); phase={phase}; last_progress_tick={last_progress_tick}; timeout is not match success and does not prove a deadlock"
    )
}

fn failure_document(
    kind: &str,
    message: &str,
    profile_hz: u32,
    seed: u64,
    map_id: &Option<String>,
    waves: u32,
    max_game_seconds: u64,
    stall_game_seconds: u64,
    max_ticks: u64,
    stall_ticks: u64,
    last_progress_tick: u64,
    observed_tick: u64,
    samples: &[Value],
) -> Value {
    json!({
        "success": false,
        "match_played": false,
        "failure_kind": kind,
        "scope": FAILURE_SCOPE,
        "step_wall_ms_mean": step_wall_ms_mean(),
        "visibility_wave_ms_mean": visibility_wall_ms_mean(),
        "error": message,
        "budget_scope": BUDGET_SCOPE,
        "stall_scope": STALL_SCOPE,
        "max_game_seconds": max_game_seconds,
        "stall_game_seconds": stall_game_seconds,
        "max_ticks": max_ticks,
        "stall_ticks": stall_ticks,
        "last_progress_tick": last_progress_tick,
        "observed_tick": observed_tick,
        "profile_hz": profile_hz,
        "seed": seed,
        "map_id": map_id,
        "waves": waves,
        "samples": samples,
    })
}

fn diagnostic_sample_due(tick: u64, ticks_per_second: u32) -> bool {
    match u64::from(ticks_per_second).checked_mul(DIAGNOSTIC_SAMPLE_INTERVAL_SECONDS) {
        Some(interval) if interval > 0 => tick > 0 && tick.is_multiple_of(interval),
        _ => false,
    }
}

fn remember_diagnostic_sample(samples: &mut Vec<Value>, sample: Value) {
    let tick = sample.get("tick").and_then(Value::as_u64);
    if tick.is_some() && samples.last().and_then(|existing| existing.get("tick")).and_then(Value::as_u64) == tick {
        samples.pop();
    }
    if samples.len() >= DIAGNOSTIC_SAMPLE_LIMIT {
        samples.remove(0);
    }
    samples.push(sample);
}

fn consider_diagnostic_tick(samples: &mut Vec<u64>, tick: u64, ticks_per_second: u32, force_final: bool) {
    if !force_final && !diagnostic_sample_due(tick, ticks_per_second) {
        return;
    }
    if samples.last().copied() == Some(tick) {
        return;
    }
    if samples.len() >= DIAGNOSTIC_SAMPLE_LIMIT {
        samples.remove(0);
    }
    samples.push(tick);
}

/// Positions, commands, living structures and lane counts only. No private
/// enemy orders are fed back to bots. Camp timers and full creep lists stay out.
fn bounded_match_snapshot(world: &specs::World, bots: Option<&RoleBotConfig>, tick: u64) -> Value {
    let xy = |pos: omoba_sim::Vec2| json!({"x_raw": pos.x.raw(), "y_raw": pos.y.raw()});
    let living_creeps: Vec<(i32, String, omoba_sim::Vec2)> = {
        let creeps = world.read_storage::<Creep>();
        let positions = world.read_storage::<Pos>();
        let properties = world.read_storage::<CProperty>();
        let factions = world.read_storage::<Faction>();
        (&creeps, &positions, &properties, &factions).join().filter_map(|(creep, pos, prop, faction)| {
            (prop.hp.raw() > 0).then(|| (faction.team_id, creep.path.clone(), pos.0))
        }).collect()
    };
    let state = world.read_resource::<MobaMatch>();
    let positions = world.read_storage::<Pos>();
    let properties = world.read_storage::<CProperty>();
    let commands = world.read_storage::<HeroCommandQueue>();
    let alive = world.entities();
    let map_lanes = state.config.map_id.as_deref().and_then(omoba_template_ids::moba_map_by_name)
        .map(|map| (0..map.lane_count()).filter_map(|lane| map.lane_id(lane)).map(str::to_string).collect::<Vec<_>>())
        .unwrap_or_default();
    let lane_name = |lane: usize| map_lanes.get(lane).cloned().unwrap_or_else(|| lane.to_string());
    let vitals = |entity: specs::Entity| properties.get(entity).map(|prop| json!({
        "hp_raw": prop.hp.raw(), "max_hp_raw": prop.mhp.raw(),
    }));
    let mut heroes = Vec::new();
    for slot in &state.heroes {
        let assignment = bots.and_then(|bots| bots.assignments.iter().find(|assignment| assignment.player_id == slot.player_id));
        let entity = slot.entity.filter(|entity| alive.is_alive(*entity));
        heroes.push(json!({
            "player_id": slot.player_id,
            "team_id": state.config.teams[slot.side],
            "role": assignment.map(|assignment| format!("{:?}", assignment.role)),
            "lane": assignment.map(|assignment| lane_name(assignment.lane)),
            "alive": entity.is_some(),
            "deaths": slot.deaths,
            "kills": slot.kills,
            "respawns": slot.respawns,
            "position": entity.and_then(|entity| positions.get(entity).map(|pos| xy(pos.0))),
            "vitals": entity.and_then(vitals),
            "command": entity.and_then(|entity| commands.get(entity)).map(command_summary).unwrap_or(json!({"kind":"none"})),
        }));
    }
    let mut towers = Vec::new();
    for (lane, sides) in state.lane_towers.iter().enumerate() {
        for (side, tower) in sides.iter().enumerate() {
            let entity = tower.filter(|entity| alive.is_alive(*entity));
            let position = entity.and_then(|entity| positions.get(entity).map(|pos| pos.0));
            let team_id = state.config.teams[side] as i32;
            towers.push(json!({
                "lane": lane_name(lane),
                "team_id": state.config.teams[side],
                "alive": entity.is_some(),
                "position": position.map(xy),
                "vitals": entity.and_then(vitals),
                "nearest_enemy_creep_distance_sq_raw": position.map(|origin| living_creeps.iter()
                    .filter(|(team, _, _)| *team != team_id && *team != 0)
                    .map(|(_, _, pos)| (*pos - origin).length_squared().raw()).min()),
            }));
        }
    }
    let mut bases = Vec::new();
    for (side, base) in state.bases.iter().enumerate() {
        let entity = base.filter(|entity| alive.is_alive(*entity));
        bases.push(json!({
            "team_id": state.config.teams[side],
            "alive": entity.is_some(),
            "position": entity.and_then(|entity| positions.get(entity).map(|pos| xy(pos.0))),
            "vitals": entity.and_then(vitals),
        }));
    }
    let mut lanes = Vec::new();
    for lane in 0..state.lane_towers.len() {
        for side in 0..2 {
            let team_id = state.config.teams[side];
            let prefix = format!("__moba_lane_{lane}__");
            let count = living_creeps.iter().filter(|(team, path, _)| *team == team_id as i32 && path == &prefix).count();
            lanes.push(json!({"lane": lane_name(lane), "team_id": team_id, "living_creeps": count}));
        }
    }
    // Soldier camps and general towers (E187): what still guards each base.
    let base_structures: Vec<_> = state.base_structure_rows().into_iter().map(|(team, lane, role, entity, respawn_at, unlocked)| {
        let entity = entity.filter(|entity| alive.is_alive(*entity));
        json!({
            "team_id": state.config.teams[team],
            "lane": lane_name(lane),
            "role": format!("{role:?}"),
            "alive": entity.is_some(),
            "attackable": unlocked,
            "respawn_at_raw": respawn_at.map(|at| at.raw()),
            "vitals": entity.and_then(vitals),
        })
    }).collect();
    let camps: Vec<_> = state.jungle_camps.iter().map(|camp| {
        let entity = camp.entity.filter(|entity| alive.is_alive(*entity));
        json!({
            "id": camp.definition.id,
            "alive": entity.is_some(),
            "respawns": camp.respawns,
            "position": entity.and_then(|entity| positions.get(entity).map(|pos| xy(pos.0))),
            "vitals": entity.and_then(vitals),
        })
    }).collect();
    json!({
        "tick": tick,
        "phase": format!("{:?}", state.phase),
        "waves": state.waves,
        "heroes": heroes,
        "towers": towers,
        "bases": bases,
        "lanes": lanes,
        "base_structures": base_structures,
        "camps": camps,
    })
}

fn command_summary(queue: &HeroCommandQueue) -> Value {
    let queued = queue.queued.len();
    match queue.active {
        None => json!({"kind":"none","queued":queued}),
        Some(HeroCommand::HoldPosition) => json!({"kind":"hold","queued":queued}),
        Some(HeroCommand::MoveTo { pos }) => json!({"kind":"move","x_raw":pos.x.raw(),"y_raw":pos.y.raw(),"queued":queued}),
        Some(HeroCommand::AttackMove { pos }) => json!({"kind":"attack_move","x_raw":pos.x.raw(),"y_raw":pos.y.raw(),"queued":queued}),
        Some(HeroCommand::AttackTarget { target, .. }) => json!({"kind":"attack","target":target.id(),"gen":target.gen().id(),"queued":queued}),
    }
}

/// Internal side stays the finished side index. `winner_team` follows the public game.end mapping.
fn reported_match_winner(teams: &[u32; 2], winner_side: Option<u8>) -> Result<(Option<u8>, Option<u32>), Error> {
    let winner_team = match winner_side {
        None => None,
        Some(side) => Some(*teams.get(usize::from(side)).ok_or_else(|| {
            err_msg(format!("illegal winner side {side}"))
        })?),
    };
    Ok((winner_side, winner_team))
}

fn paired_failure_samples_path(report: &Path) -> PathBuf {
    let stem = report.file_stem().and_then(|stem| stem.to_str()).unwrap_or("report");
    report.with_file_name(format!("{stem}.failure-samples.json"))
}

fn reject_existing_outputs(report: &Path) -> Result<(), Error> {
    if report.exists() {
        return Err(err_msg(format!(
            "refusing to overwrite existing report {}",
            report.display()
        )));
    }
    let paired = paired_failure_samples_path(report);
    if paired.exists() {
        return Err(err_msg(format!(
            "refusing to overwrite existing diagnostic {}",
            paired.display()
        )));
    }
    Ok(())
}

fn write_new_output(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            err_msg(format!(
                "refusing to overwrite existing output {}: {error}",
                path.display()
            ))
        })?;
    file.write_all(bytes)?;
    Ok(())
}

fn write_failure(report: &Path, document: &Value) -> Result<(), Error> {
    let bytes = serde_json::to_vec_pretty(document)?;
    write_new_output(report, &bytes)?;
    write_new_output(&paired_failure_samples_path(report), &bytes)?;
    let message = document
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("headless failure")
        .to_string();
    Err(err_msg(message))
}

// Gameplay log lines (for example the "[match]" structure, death and result lines) on stderr when
// RUST_LOG is set; the batch runner keeps stderr as each match's log (E154). Off by default.
fn init_logging() {
    let Ok(level) = std::env::var("RUST_LOG") else { return };
    use log4rs::append::console::{ConsoleAppender, Target};
    use log4rs::config::{Appender, Config, Root};
    let level = level.parse().unwrap_or(log::LevelFilter::Info);
    let stderr = ConsoleAppender::builder().target(Target::Stderr).build();
    if let Ok(config) = Config::builder()
        .appender(Appender::builder().build("stderr", Box::new(stderr)))
        .build(Root::builder().appender("stderr").build(level))
    {
        let _ = log4rs::init_config(config);
    }
}

fn main() {
    init_logging();
    if let Err(error) = run() {
        eprintln!("MOBA headless FAILED: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    fn slot(kind: ObjectiveKind, layer: u16, entity: u32, hp: i64) -> ObjectiveSlot {
        slot_at(kind, layer, entity, if entity == 0 { 0 } else { 1 }, hp)
    }

    fn slot_at(kind: ObjectiveKind, layer: u16, entity: u32, gen: i32, hp: i64) -> ObjectiveSlot {
        ObjectiveSlot {
            kind,
            lane: 0,
            layer,
            team_id: 1,
            entity_id: entity,
            entity_gen: gen,
            hp_raw: hp,
        }
    }

    fn observation(objectives: Vec<ObjectiveSlot>) -> HeadlessObservation {
        HeadlessObservation {
            objectives,
            waves: 1,
            hero_moves: 0,
            hero_kills: 0,
            camp_respawns: 0,
            private_commands: 0,
        }
    }

    #[test]
    fn budget_options_reject_invalid_overflow_and_duplicates() {
        assert_eq!(parse_budget_seconds("60", "max-game-seconds").unwrap(), 60);
        assert_eq!(parse_budget_seconds("3600", "max-game-seconds").unwrap(), 3600);
        for bad in [
            "", "0", "00", "59", "3601", "0600", "-60", "+60", "60.0", "1e2", " 60", "600 ", "abc",
        ] {
            assert!(parse_budget_seconds(bad, "max-game-seconds").is_err(), "{bad}");
        }
        let overflow = parse_budget_seconds("18446744073709551616", "max-game-seconds").unwrap_err();
        assert!(format!("{overflow}").contains("overflow"), "{overflow}");
        assert!(parse_budget_seconds("999999999999999999999", "stall-game-seconds").unwrap_err().to_string().contains("overflow"));
        assert_eq!(checked_game_ticks(600, 60).unwrap(), 36_000);
        assert_eq!(checked_game_ticks(3600, 120).unwrap(), 432_000);
        assert_eq!(checked_game_ticks(60, 15).unwrap(), 900);
        assert!(format!("{}", checked_game_ticks(u64::MAX, 60).unwrap_err()).contains("overflow"));
        assert!(checked_game_ticks(u64::MAX / 60 + 1, 60).is_err());

        // E096: --map carries the map's hero XP multiplier like its tower and base hp.
        for map in omoba_template_ids::MOBA_MAPS {
            let parsed = parse_headless_args(["--map", map.id]).unwrap();
            assert_eq!(parsed.config.hero_xp_scale_permille, map.hero_xp_scale_permille(), "{}", map.id);
        }
        let defaults = parse_headless_args(Vec::<&str>::new()).unwrap();
        assert_eq!(defaults.max_game_seconds, 600);
        assert_eq!(defaults.stall_game_seconds, 300);
        let explicit = parse_headless_args(["--max-game-seconds", "1800", "--stall-game-seconds", "300"]).unwrap();
        assert_eq!(explicit.max_game_seconds, 1800);
        assert_eq!(explicit.stall_game_seconds, 300);
        assert_eq!(checked_game_ticks(explicit.max_game_seconds, 60).unwrap(), 108_000);
        parse_headless_args(["--max-game-seconds", "60", "--stall-game-seconds", "60"]).unwrap();
        parse_headless_args(["--max-game-seconds", "3600", "--stall-game-seconds", "3600"]).unwrap();
        for args in [
            vec!["--max-game-seconds", "600", "--max-game-seconds", "700"],
            vec!["--stall-game-seconds", "300", "--stall-game-seconds", "300"],
            vec!["--report", "a.json", "--report", "b.json"],
            vec!["--max-game-seconds", "100"],
            vec!["--max-game-seconds", "60", "--stall-game-seconds", "61"],
            vec!["--stall-game-seconds", "400", "--max-game-seconds", "300"],
            vec!["--max-game-seconds", "60.0"],
            vec!["--stall-game-seconds"],
        ] {
            assert!(parse_headless_args(args).is_err());
        }
    }

    #[test]
    fn objective_progress_is_hp_retirement_or_layer_replacement_only() {
        let before = observation(vec![
            slot(ObjectiveKind::TowerLayer, 0, 10, 100),
            slot(ObjectiveKind::TowerLayer, 1, 11, 500),
            slot(ObjectiveKind::TowerFront, 0, 10, 100),
            slot(ObjectiveKind::Base, 0, 20, 1_000),
        ]);
        let mut hp = observation(before.objectives.clone());
        hp.objectives[0].hp_raw = 99;
        assert!(objective_progressed(&before, &hp));
        let mut base_hp = observation(before.objectives.clone());
        base_hp.objectives[3].hp_raw = 999;
        assert!(objective_progressed(&before, &base_hp));
        // E187: soldier camps and general towers are objectives too (damage and retirement).
        let structures = observation(vec![slot(ObjectiveKind::BaseStructure, 0, 30, 3_000)]);
        let mut camp_hp = observation(structures.objectives.clone());
        camp_hp.objectives[0].hp_raw = 2_999;
        assert!(objective_progressed(&structures, &camp_hp));
        let camp_down = observation(vec![slot(ObjectiveKind::BaseStructure, 0, 0, 0)]);
        assert!(objective_progressed(&structures, &camp_down));

        let mut retired = observation(before.objectives.clone());
        retired.objectives[0] = slot(ObjectiveKind::TowerLayer, 0, 0, 0);
        retired.objectives[2] = slot(ObjectiveKind::TowerFront, 0, 11, 500);
        assert!(objective_progressed(&before, &retired));

        let mut replaced = observation(before.objectives.clone());
        replaced.objectives[2] = slot(ObjectiveKind::TowerFront, 0, 11, 500);
        assert!(objective_progressed(&before, &replaced));
        let appeared = observation(vec![slot(ObjectiveKind::TowerLayer, 0, 0, 0)]);
        let filled = observation(vec![slot(ObjectiveKind::TowerLayer, 0, 11, 500)]);
        assert!(objective_progressed(&appeared, &filled));

        let mut healed = observation(before.objectives.clone());
        healed.objectives[0].hp_raw = 101;
        assert!(!objective_progressed(&before, &healed));
        assert!(!objective_progressed(&before, &observation(before.objectives.clone())));

        let mut noise = observation(before.objectives.clone());
        noise.waves = 40;
        noise.hero_moves = 9;
        noise.hero_kills = 7;
        noise.camp_respawns = 4;
        noise.private_commands = 3;
        assert!(!objective_progressed(&before, &noise));

        let many: Vec<_> = (0..MAX_OBJECTIVE_SLOTS as u16 + 1)
            .map(|layer| slot(ObjectiveKind::TowerLayer, layer, 1, 10))
            .collect();
        assert_eq!(bound_objective_slots(many.clone()).len(), MAX_OBJECTIVE_SLOTS);
        let mut dropped = many.clone();
        dropped[MAX_OBJECTIVE_SLOTS].hp_raw = 9;
        assert!(!objective_slots_progressed(&many, &dropped));
        let mut kept = many.clone();
        kept[0].hp_raw = 9;
        assert!(objective_slots_progressed(&many, &kept));

        let living = observation(vec![slot_at(ObjectiveKind::TowerLayer, 0, 0, 1, 100)]);
        let mut damaged = observation(living.objectives.clone());
        damaged.objectives[0].hp_raw = 99;
        assert!(objective_progressed(&living, &damaged));
        let retired = observation(vec![slot_at(ObjectiveKind::TowerLayer, 0, 0, 0, 0)]);
        assert!(objective_progressed(&living, &retired));
        let replaced = observation(vec![slot_at(ObjectiveKind::TowerLayer, 0, 0, 2, 500)]);
        assert!(objective_progressed(&living, &replaced));
        let empty = observation(vec![slot_at(ObjectiveKind::TowerLayer, 0, 0, 0, 0)]);
        let empty_noise = observation(vec![slot_at(ObjectiveKind::TowerLayer, 0, 5, 0, 40)]);
        assert!(!objective_progressed(&empty, &empty_noise));
        assert!(!objective_progressed(&empty, &observation(vec![slot_at(ObjectiveKind::TowerLayer, 0, 0, 0, 0)])));
        assert!(!objective_progressed(&living, &observation(living.objectives.clone())));
    }

    #[test]
    fn winner_team_maps_config_teams_and_rejects_an_illegal_side() {
        let teams = [7, 11];
        let (side, team) = reported_match_winner(&teams, Some(0)).unwrap();
        assert_eq!((side, team), (Some(0), Some(7)));
        assert_eq!(
            json!({"winner_side": side, "winner_team": team}),
            json!({"winner_side": 0, "winner_team": 7})
        );
        let (side, team) = reported_match_winner(&teams, Some(1)).unwrap();
        assert_eq!((side, team), (Some(1), Some(11)));
        let (side, team) = reported_match_winner(&teams, None).unwrap();
        assert_eq!((side, team), (None, None));
        assert_eq!(
            json!({"winner_side": side, "winner_team": team}),
            json!({"winner_side": null, "winner_team": null})
        );
        let error = reported_match_winner(&teams, Some(2)).unwrap_err();
        assert!(format!("{error}").contains("illegal winner side 2"), "{error}");
    }

    #[test]
    fn objective_stall_boundary_uses_checked_tick_budget() {
        let stall = checked_game_ticks(300, 60).unwrap();
        assert_eq!(stall, 18_000);
        let base = vec![slot(ObjectiveKind::TowerLayer, 0, 10, 100)];
        let mut quiet = ObjectiveMonitor::baseline(base.clone());
        assert!(quiet.observe(stall - 1, base.clone(), stall).is_none());
        assert_eq!(quiet.observe(stall, base.clone(), stall), Some(0));

        let mut moving = ObjectiveMonitor::baseline(base.clone());
        assert!(moving.observe(stall - 1, base.clone(), stall).is_none());
        let mut hit = base.clone();
        hit[0].hp_raw = 99;
        assert!(moving.observe(stall, hit.clone(), stall).is_none());
        assert_eq!(moving.last_progress_tick, stall);
        assert!(moving.observe(stall + stall - 1, hit.clone(), stall).is_none());
        assert_eq!(moving.observe(stall + stall, hit, stall), Some(stall));

        let mut waves_only = ObjectiveMonitor::baseline(base.clone());
        assert_eq!(waves_only.observe(stall, base, stall), Some(0));
    }

    #[test]
    fn diagnostic_samples_keep_bounded_recent_history_and_final_tick() {
        let mut samples = Vec::new();
        let hz = 120u32;
        let last_second = 3_600u64;
        for tick in 0..=hz as u64 * last_second {
            consider_diagnostic_tick(&mut samples, tick, hz, false);
            assert!(samples.len() <= DIAGNOSTIC_SAMPLE_LIMIT);
        }
        assert_eq!(samples.len(), DIAGNOSTIC_SAMPLE_LIMIT);
        assert_eq!(*samples.last().unwrap(), hz as u64 * last_second);
        assert_eq!(
            samples[0],
            hz as u64 * last_second - (DIAGNOSTIC_SAMPLE_LIMIT as u64 - 1) * hz as u64 * DIAGNOSTIC_SAMPLE_INTERVAL_SECONDS
        );
        assert!(samples.len() < 1_000);
        let final_tick = hz as u64 * last_second + 7;
        consider_diagnostic_tick(&mut samples, final_tick, hz, true);
        assert_eq!(samples.len(), DIAGNOSTIC_SAMPLE_LIMIT);
        assert_eq!(*samples.last().unwrap(), final_tick);
        let frozen = samples.clone();
        consider_diagnostic_tick(&mut samples, final_tick, hz, true);
        consider_diagnostic_tick(&mut samples, final_tick + 1, hz, false);
        assert_eq!(samples, frozen);
        assert!(!diagnostic_sample_due(0, 60));
        assert!(!diagnostic_sample_due(3_599, 60));
        assert!(diagnostic_sample_due(3_600, 60));
        assert!(!diagnostic_sample_due(1, u32::MAX));
    }

    #[test]
    fn stall_and_timeout_documents_are_not_acceptance() {
        let stall = failure_document(
            "objective_stall",
            &stall_message(300, 12, 18_012),
            60,
            1,
            &Some("three_lane".into()),
            4,
            1_800,
            300,
            108_000,
            18_000,
            12,
            18_012,
            &[json!({"tick": 18012})],
        );
        let timeout = failure_document(
            "execution_budget",
            &timeout_message(1_800, 108_000, 100_000, "Playing"),
            60,
            1,
            &None,
            8,
            1_800,
            300,
            108_000,
            18_000,
            100_000,
            108_000,
            &[json!({"tick": 108000})],
        );
        for document in [&stall, &timeout] {
            assert_eq!(document["success"], false);
            assert_eq!(document["match_played"], false);
            assert_eq!(document["budget_scope"], BUDGET_SCOPE);
            assert_eq!(document["stall_scope"], STALL_SCOPE);
            assert_eq!(document["max_game_seconds"], 1_800);
            assert_eq!(document["stall_game_seconds"], 300);
            assert!(document["samples"].as_array().unwrap().len() <= DIAGNOSTIC_SAMPLE_LIMIT);
        }
        assert!(stall["error"].as_str().unwrap().contains("last_progress_tick=12"));
        assert!(timeout["error"].as_str().unwrap().contains("does not prove a deadlock"));
        assert_eq!(timeout["failure_kind"], "execution_budget");
        assert_eq!(stall["samples"][0]["tick"].as_u64(), Some(18_012));
    }

    #[test]
    fn existing_failure_success_and_invalid_json_reject_overwrite() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let mut dir = None;
        for suffix in 0..1000u32 {
            let candidate = std::env::temp_dir().join(format!(
                "moba-headless-create-new-{}-{nanos}-{suffix}",
                std::process::id()
            ));
            match std::fs::create_dir(&candidate) {
                Ok(()) => {
                    dir = Some(candidate);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot reserve test directory: {error}"),
            }
        }
        let dir = dir.expect("cannot reserve test directory");
        let mut created = Vec::new();
        let cases = [
            ("success.json", &br#"{"success":true,"profile_hz":60}"#[..]),
            ("failure.json", &br#"{"success":false,"error":"old"}"#[..]),
            ("invalid.json", &b"not-json{{{"[..]),
        ];
        for (name, bytes) in cases {
            let report = dir.join(name);
            std::fs::write(&report, bytes).unwrap();
            created.push(report.clone());
            let error = reject_existing_outputs(&report).unwrap_err();
            assert!(format!("{error}").contains("refusing to overwrite"), "{error}");
            assert_eq!(std::fs::read(&report).unwrap(), bytes);
            let write_error = write_new_output(&report, b"replacement").unwrap_err();
            assert!(format!("{write_error}").contains("refusing to overwrite"), "{write_error}");
            assert_eq!(std::fs::read(&report).unwrap(), bytes);
            assert!(!paired_failure_samples_path(&report).exists());
        }
        let fresh = dir.join("fresh-result.json");
        let paired = paired_failure_samples_path(&fresh);
        assert_eq!(
            paired,
            dir.join("fresh-result.failure-samples.json")
        );
        let paired_bytes = br#"{"success":false,"samples":[]}"#;
        std::fs::write(&paired, paired_bytes).unwrap();
        created.push(paired.clone());
        let error = reject_existing_outputs(&fresh).unwrap_err();
        assert!(format!("{error}").contains("diagnostic"), "{error}");
        assert_eq!(std::fs::read(&paired).unwrap(), paired_bytes);
        assert!(!fresh.exists());
        let clean = dir.join("clean-result.json");
        assert!(reject_existing_outputs(&clean).is_ok());
        write_new_output(&clean, b"{\"success\":false}").unwrap();
        created.push(clean.clone());
        assert!(write_new_output(&clean, b"newer").is_err());
        assert_eq!(std::fs::read(&clean).unwrap(), b"{\"success\":false}");
        for path in created {
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(&dir).unwrap();
    }
}

/// The trace resources the statistics read: casts (CastTrace), settled damage (DamageTally) and
/// script damage by ability (DamageTrace, E436: the generated code then tags each packet). The
/// replay world gets them too (E446): with a DamageTrace the scripts emit tags, and a replay
/// without one diverged from the recorded run (every quick_swap of M6 batch 3 at tick 1077).
fn insert_stat_traces(world: &mut specs::World) {
    world.insert(omoba_core::runtime::native::cast_trace::CastTrace::default());
    world.insert(omoba_core::runtime::native::damage_tally::DamageTally::default());
    world.insert(omoba_core::runtime::native::damage_trace::DamageTrace::default());
}

/// A replay keeps no statistics: its traces are emptied every tick.
fn clear_stat_traces(world: &specs::World) {
    world.write_resource::<omoba_core::runtime::native::cast_trace::CastTrace>().0.clear();
    world.write_resource::<omoba_core::runtime::native::damage_tally::DamageTally>().0.clear();
    world.write_resource::<omoba_core::runtime::native::damage_trace::DamageTrace>().0.clear();
}

/// Per-tick cost of the run (E512): every formal tick's wall time (driver step plus the bots'
/// visibility wave), its distribution against the tick budget (1/hz), and for the slowest ticks
/// the systems and phases that took the time, from the world's TickProfile counters.
struct TickPerf {
    budget_ns: u64,
    started: std::time::Instant,
    samples: Vec<u64>,
    previous: std::collections::BTreeMap<&'static str, u128>,
    /// (ns, tick, heaviest parts), the TICK_PERF_SLOWEST slowest.
    slowest: Vec<(u64, u64, Vec<(&'static str, u64)>)>,
}

const TICK_PERF_SLOWEST: usize = 10;
const TICK_PERF_PARTS: usize = 4;

impl TickPerf {
    fn new(hz: u32) -> Self {
        Self { budget_ns: 1_000_000_000 / u64::from(hz.max(1)), started: std::time::Instant::now(), samples: Vec::new(),
            previous: Default::default(), slowest: Vec::new() }
    }

    /// The TickProfile counters, cumulative: every system, and the dispatch phases.
    fn counters(world: &specs::World) -> std::collections::BTreeMap<&'static str, u128> {
        let Some(profile) = world.try_fetch::<omoba_core::comp::TickProfile>() else { return Default::default() };
        let mut out: std::collections::BTreeMap<&'static str, u128> = profile.system_stats.lock()
            .map(|stats| stats.iter().map(|(name, stat)| (*name, stat.ns)).collect()).unwrap_or_default();
        out.insert("phase:script_dispatch", profile.script_dispatch_ns);
        out.insert("phase:process_outcomes", profile.process_outcomes_ns);
        out
    }

    fn record(&mut self, world: &specs::World, tick: u64, ns: u64) {
        self.samples.push(ns);
        let current = Self::counters(world);
        let slow = self.slowest.len() < TICK_PERF_SLOWEST || self.slowest.last().is_some_and(|(least, _, _)| ns > *least);
        if slow {
            // A counter that went down was reset by its log window: this tick's part is all of it.
            let mut parts: Vec<(&'static str, u64)> = current.iter().map(|(name, now)| {
                let before = self.previous.get(name).copied().unwrap_or(0);
                (*name, (if *now >= before { now - before } else { *now }) as u64)
            }).filter(|(_, part)| *part > 0).collect();
            parts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
            parts.truncate(TICK_PERF_PARTS);
            self.slowest.push((ns, tick, parts));
            self.slowest.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            self.slowest.truncate(TICK_PERF_SLOWEST);
        }
        self.previous = current;
    }

    fn report(&self) -> serde_json::Value {
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();
        let ms = |ns: u64| (ns as f64 / 1e4).round() / 100.0;
        let at = |q: f64| sorted.get(((sorted.len() as f64 - 1.0) * q).round() as usize).copied().map_or(0.0, ms);
        serde_json::json!({"ticks": sorted.len(), "budget_ms": ms(self.budget_ns), "p50_ms": at(0.5), "p90_ms": at(0.9),
            "p99_ms": at(0.99), "max_ms": sorted.last().copied().map_or(0.0, ms),
            "over_budget": sorted.iter().filter(|ns| **ns > self.budget_ns).count(),
            "run_wall_seconds": (self.started.elapsed().as_secs_f64() * 10.0).round() / 10.0,
            "slowest": self.slowest.iter().map(|(ns, tick, parts)| serde_json::json!({"tick": tick, "ms": ms(*ns),
                "parts": parts.iter().map(|(name, part)| serde_json::json!({"name": name, "ms": ms(*part)})).collect::<Vec<_>>()}))
                .collect::<Vec<_>>()})
    }
}

/// Per-hero, per-ability headless statistics (E265; `OMB_HEADLESS_HERO_STATS=1`), keyed by player.
#[derive(Default)]
struct HeroStats {
    /// Cast inputs by the ability the slot held when the bot chose it (E400: a swapped slot, e.g.
    /// C14 龍化 putting c14_fire_breath_dragon in c14_fire_breath's slot, credits each to its own).
    inputs: std::collections::BTreeMap<(u32, String), u32>,
    /// The slot each ability a player's hero held was last seen in (swapped-out ones included).
    slots: std::collections::BTreeMap<(u32, String), usize>,
    accepted: std::collections::BTreeMap<(u32, u64), u32>,
    summons: std::collections::BTreeMap<u32, std::collections::BTreeSet<u64>>,
    heroes: std::collections::BTreeMap<u32, (String, Vec<String>)>,
    /// Every hero entity each player had, all lives: a cast trace row names the life that cast it,
    /// which a respawn replaces (the slot holds only the current one).
    lives: std::collections::BTreeMap<specs::Entity, u32>,
    /// Shop items (E336): each player's inventory counts at the last observation, and how many of
    /// each item it obtained (a count that grew: bought, or assembled from its recipe).
    held: std::collections::BTreeMap<u32, std::collections::BTreeMap<String, u32>>,
    obtained: std::collections::BTreeMap<(u32, String), u32>,
    /// Health each player's hero removed (DamageTally, E352; E342 balance checks): from anything,
    /// and from heroes.
    damage: std::collections::BTreeMap<u32, i64>,
    hero_damage: std::collections::BTreeMap<u32, i64>,
    /// Units a player's hero summoned or copied (SummonedUnit.summoner, E436): their damage is
    /// credited to the summoner's player, as a packet's credit names the unit that dealt it.
    owned: std::collections::BTreeMap<specs::Entity, u32>,
    /// Script damage by (player, ability) from the DamageTrace (E436): applied damage, the part on
    /// heroes, and packets cancelled or skipped because the source had died.
    ability_damage: std::collections::BTreeMap<(u32, String), [i64; 4]>,
    /// The last unit target (entity index) each (player, ability) cast input named.
    cast_targets: std::collections::BTreeMap<(u32, String), u32>,
    /// CastTrace rows already sampled.
    trace_seen: usize,
    /// INVALID_TARGET casts with the target's state on that tick (E454), INVALID_TARGET_SAMPLES
    /// per (player, ability): the executor can tell what made the target invalid.
    invalid_targets: std::collections::BTreeMap<(u32, String), Vec<serde_json::Value>>,
    /// Lane creeps facing where they walk or attack (E461).
    creep_facing: CreepFacing,
}

/// Creep facing from the simulation data (E461): every CREEP_FACING_INTERVAL ticks a walking creep
/// (moved at least CREEP_FACING_MIN_STEP since the last sample) must face its last tick's step, and a
/// standing creep whose attack wound up since then must face its target, both within
/// CREEP_FACING_MAX_DEGREES. Screenshots cannot show this reliably; the batch and split accept on
/// 0 violations.
/// The expected walking direction is the last tick's step, not the window's (E471): a creep turning
/// at a corner faces its new heading at once while the window still averages the old one.
#[derive(Default)]
struct CreepFacing {
    last: std::collections::BTreeMap<specs::Entity, (u64, omoba_sim::Vec2)>,
    /// Every creep's position on the previous tick.
    previous: std::collections::BTreeMap<specs::Entity, omoba_sim::Vec2>,
    walk_checks: u64,
    attack_checks: u64,
    violations: u64,
    samples: Vec<serde_json::Value>,
}

/// What an attack target is, for the creep facing samples (E476).
fn unit_kind(world: &specs::World, entity: specs::Entity) -> &'static str {
    if world.read_storage::<omoba_core::runtime::Hero>().get(entity).is_some() { "hero" }
    else if world.read_storage::<omoba_core::runtime::Tower>().get(entity).is_some() { "structure" }
    else if world.read_storage::<omoba_core::runtime::SummonedUnit>().get(entity).is_some() { "summon" }
    else if world.read_storage::<omoba_core::runtime::Creep>().get(entity).is_some() { "creep" }
    // a lane structure without a Tower component, e.g. the base (E491)
    else if world.try_fetch::<MobaMatch>().and_then(|state| state.lane_role(entity))
        .is_some_and(|role| role != omoba_core::runtime::LaneRole::Creep) { "structure" }
    else { "other" }
}

const CREEP_FACING_INTERVAL: u64 = 30;
const CREEP_STACK_DISTANCE: i32 = 24;
const CREEP_FACING_MIN_STEP: f32 = 20.0;
const CREEP_FACING_MAX_DEGREES: f32 = 30.0;
const CREEP_FACING_SAMPLES: usize = 10;

impl CreepFacing {
    fn observe(&mut self, world: &specs::World) {
        use specs::Join;
        let tick = world.read_resource::<omoba_core::runtime::Tick>().0;
        let previous = std::mem::take(&mut self.previous);
        self.previous = (&world.entities(), &world.read_storage::<omoba_core::runtime::Creep>(), &world.read_storage::<Pos>()).join()
            .map(|(entity, _, pos)| (entity, pos.0)).collect();
        if tick % CREEP_FACING_INTERVAL != 0 { return; }
        let degrees = |v: omoba_sim::Vec2| v.y.to_f32_for_render().atan2(v.x.to_f32_for_render()).to_degrees();
        let attacks = world.try_fetch::<omoba_core::runtime::native::rules::attack::AttackRecords>();
        let state = world.try_fetch::<MobaMatch>();
        let radii = world.read_storage::<omoba_core::runtime::CollisionRadius>();
        let radius = |unit: specs::Entity| radii.get(unit).map_or(omoba_sim::Fixed64::ZERO, |r| r.0);
        let buffs = world.read_resource::<omoba_core::runtime::BuffStore>();
        let creeps = world.read_storage::<omoba_core::runtime::Creep>();
        let positions = world.read_storage::<Pos>();
        let properties = world.read_storage::<CProperty>();
        let mut seen = std::collections::BTreeMap::new();
        for (entity, creep, pos, facing) in (&world.entities(), &creeps,
            &positions, &world.read_storage::<omoba_core::runtime::Facing>()).join() {
            if properties.get(entity).is_none_or(|p| p.hp <= omoba_sim::Fixed64::ZERO) { continue; }
            seen.insert(entity, (tick, pos.0));
            let Some(&(before, from)) = self.last.get(&entity) else { continue };
            // A stunned creep neither moves nor turns, and forced motion moves it without turning
            // (E510): neither is the creep's own facing.
            if buffs.is_stunned(entity) || buffs.has(entity, omoba_core::runtime::native::forced_motion::BUFF) { continue; }
            let step = pos.0 - from;
            let walked = step.length().to_f32_for_render() >= CREEP_FACING_MIN_STEP;
            let last_step = previous.get(&entity).map(|from| pos.0 - *from)
                .filter(|step| step.length() > omoba_sim::Fixed64::ZERO);
            // Attacking (E491): the creep's current lane target, alive and within its attack range (a
            // farther one is chased, which is walking), not the last attack record's target, which the
            // creep may already have left. Overlapping bodies (within both collision radii) have no
            // comparable direction (E476: atan2(0, 0) read as 0°).
            let expected = if walked { last_step.map(|step| ("walk", degrees(step), None)) } else {
                state.as_ref().and_then(|state| state.lane_attack(world, entity))
                    .filter(|(target, _)| world.entities().is_alive(*target)
                        && properties.get(*target).is_some_and(|p| p.hp > omoba_sim::Fixed64::ZERO))
                    .and_then(|(target, range)| positions.get(target).map(|at| (target, at.0, range)))
                    .filter(|(target, at, range)| {
                        let distance = (*at - pos.0).length();
                        // lane creeps have no body radius and stack: two creeps within one tick of
                        // movement (about 11 each) have no stable direction (E510)
                        let stacked = creeps.get(*target).is_some() && distance < omoba_sim::Fixed64::from_i32(CREEP_STACK_DISTANCE);
                        distance <= *range && distance >= radius(entity) + radius(*target) && !stacked
                    })
                    .map(|(target, at, _)| ("attack", degrees(at - pos.0), Some((target, at))))
            };
            let Some((mode, expected, target)) = expected else { continue };
            if mode == "walk" { self.walk_checks += 1 } else { self.attack_checks += 1 }
            let facing = facing.rad_f32().to_degrees();
            let off = ((facing - expected).rem_euclid(360.0) + 180.0).rem_euclid(360.0) - 180.0;
            if off.abs() > CREEP_FACING_MAX_DEGREES {
                self.violations += 1;
                if self.samples.len() < CREEP_FACING_SAMPLES {
                    self.samples.push(serde_json::json!({"tick": tick, "creep": creep.name, "entity": entity.id(), "mode": mode,
                        "facing": facing.round(), "expected": expected.round(), "off": off.round(),
                        "x": pos.0.x.to_f32_for_render().round(), "y": pos.0.y.to_f32_for_render().round(),
                        "statuses": buffs.iter_for(entity).map(|(key, _)| key.to_string()).collect::<Vec<_>>(),
                        "target": target.map(|(target, at)| serde_json::json!({"entity": target.id(), "self": target == entity,
                            "kind": unit_kind(world, target), "last_attack": attacks.as_ref()
                                .and_then(|records| records.0.get(&entity)).and_then(|record| record.target).map(|last| last.id()), "x": at.x.to_f32_for_render().round(), "y": at.y.to_f32_for_render().round(),
                            "distance": (at - pos.0).length().to_f32_for_render().round()}))}));
                }
            }
        }
        self.last = seen;
    }

    fn report(&self) -> serde_json::Value {
        serde_json::json!({"walk_checks": self.walk_checks, "attack_checks": self.attack_checks, "violations": self.violations,
            "max_degrees": CREEP_FACING_MAX_DEGREES, "samples": self.samples})
    }
}

const INVALID_TARGET_SAMPLES: usize = 5;

impl HeroStats {
    /// The target state of an INVALID_TARGET cast: the unit the last cast input of this ability named.
    fn invalid_target_sample(&self, world: &specs::World, tick: u64, caster: specs::Entity, player: u32, ability: &str) -> serde_json::Value {
        use omoba_core::runtime::native::{magic_immunity, rules::vocab};
        let Some(&index) = self.cast_targets.get(&(player, ability.to_string())) else {
            return serde_json::json!({"tick": tick, "target": null});
        };
        let entity = world.entities().entity(index);
        let alive = world.entities().is_alive(entity);
        let factions = world.read_storage::<Faction>();
        let buffs = world.read_resource::<omoba_core::runtime::BuffStore>();
        let positions = world.read_storage::<Pos>();
        let distance = positions.get(caster).zip(positions.get(entity)).map(|(a, b)| (a.0 - b.0).length().to_f32_for_render().round());
        let sighted = factions.get(caster).and_then(|own| world.try_fetch::<omoba_core::runtime::TeamVisibilityRuntime>()
            .map(|vision| vision.teams.get(&(own.team_id as u32)).is_some_and(|team| team.sighted.contains(&omoba_core::runtime::canonical_entity_id(entity)))));
        serde_json::json!({"tick": tick, "target": index, "alive": alive,
            "hp": world.read_storage::<CProperty>().get(entity).map(|p| p.hp.to_f32_for_render().round()),
            "hero": world.read_storage::<omoba_core::runtime::Hero>().get(entity).is_some(),
            "hostile": factions.get(caster).zip(factions.get(entity)).map(|(a, b)| a.is_hostile_to(b)),
            "distance": distance, "sighted": sighted,
            "magic_immune_blocks": !magic_immunity::admits_target(&buffs, factions.get(caster), factions.get(entity), entity, magic_immunity::ability_pierces(ability)),
            "unselectable": vocab::unselectable(&buffs, entity), "invulnerable": vocab::invulnerable(&buffs, entity),
            "statuses": buffs.iter_for(entity).map(|(key, _)| key.to_string()).collect::<Vec<_>>()})
    }

    fn observe(&mut self, world: &specs::World, inputs: &[(u32, PlayerInput)], facts: &[omoba_core::runtime::OrderedFact]) {
        use specs::Join;
        let state = world.read_resource::<MobaMatch>();
        let heroes = world.read_storage::<omoba_core::runtime::Hero>();
        // The slot as the bot saw it: the last observation, else the hero now.
        for (player, input) in inputs {
            if let Some(PlayerInputEnum::CastAbility(cast)) = &input.action {
                let index = cast.ability_index as usize;
                let name = self.heroes.get(player).and_then(|(_, abilities)| abilities.get(index).cloned())
                    .or_else(|| state.heroes.iter().find(|slot| slot.player_id == *player).and_then(|slot| slot.entity)
                        .and_then(|entity| heroes.get(entity)).and_then(|hero| hero.abilities.get(index).cloned()))
                    .unwrap_or_else(|| format!("slot{index}"));
                if let Some(target) = cast.target_entity { self.cast_targets.insert((*player, name.clone()), target); }
                *self.inputs.entry((*player, name)).or_default() += 1;
            }
        }
        // INVALID_TARGET rows of this tick, sampled with their target's state (E454).
        let rejected: Vec<(u64, specs::Entity, String)> = world.try_fetch::<omoba_core::runtime::native::cast_trace::CastTrace>()
            .map(|trace| trace.0[self.trace_seen.min(trace.0.len())..].iter()
                .filter(|row| row.result == omoba_core::runtime::native::cast_trace::CastTraceResult::Rejected("INVALID_TARGET"))
                .map(|row| (row.tick, row.caster, row.ability.clone())).collect())
            .unwrap_or_default();
        self.trace_seen = world.try_fetch::<omoba_core::runtime::native::cast_trace::CastTrace>().map_or(0, |trace| trace.0.len());
        for (tick, caster, ability) in rejected {
            let Some(player) = self.lives.get(&caster).copied()
                .or_else(|| state.heroes.iter().find(|slot| slot.entity == Some(caster)).map(|slot| slot.player_id)) else { continue };
            if self.invalid_targets.get(&(player, ability.clone())).is_some_and(|rows| rows.len() >= INVALID_TARGET_SAMPLES) { continue; }
            let sample = self.invalid_target_sample(world, tick, caster, player, &ability);
            self.invalid_targets.entry((player, ability)).or_default().push(sample);
        }
        self.creep_facing.observe(world);
        let mut owners = std::collections::BTreeMap::new();
        for slot in &state.heroes {
            if let Some(entity) = slot.entity {
                owners.insert(omoba_core::runtime::canonical_entity_id(entity), slot.player_id);
                self.lives.insert(entity, slot.player_id);
                if let Some(hero) = heroes.get(entity) {
                    for (index, ability) in hero.abilities.iter().enumerate() {
                        self.slots.insert((slot.player_id, ability.clone()), index);
                    }
                    self.heroes.insert(slot.player_id, (hero.id.clone(), hero.abilities.clone()));
                }
            }
        }
        let inventories = world.read_storage::<omoba_core::runtime::native::comp::Inventory>();
        for slot in &state.heroes {
            let Some(inventory) = slot.entity.and_then(|entity| inventories.get(entity)) else { continue };
            let mut counts = std::collections::BTreeMap::<String, u32>::new();
            for (_, item) in inventory.items() { *counts.entry(item.item_id.clone()).or_default() += 1; }
            let before = self.held.entry(slot.player_id).or_default();
            for (id, count) in &counts {
                let grew = count.saturating_sub(before.get(id).copied().unwrap_or(0));
                if grew > 0 { *self.obtained.entry((slot.player_id, id.clone())).or_default() += grew; }
            }
            *before = counts;
        }
        let summoned = world.read_storage::<omoba_core::runtime::SummonedUnit>();
        for (entity, summon) in (&world.entities(), &summoned).join() {
            if let Some(slot) = state.heroes.iter().find(|slot| slot.entity == Some(summon.summoner)) {
                self.summons.entry(slot.player_id).or_default().insert(omoba_core::runtime::canonical_entity_id(entity));
            }
            if let Some(player) = self.lives.get(&summon.summoner).copied() { self.owned.insert(entity, player); }
        }
        // Settled damage of this tick (DamageTally, E352): a hero's credited packets by its life.
        if let Some(mut tally) = world.try_fetch_mut::<omoba_core::runtime::native::damage_tally::DamageTally>() {
            for (source, target, removed) in tally.0.drain(..) {
                if let Some(player) = self.lives.get(&source).or_else(|| self.owned.get(&source)) {
                    *self.damage.entry(*player).or_default() += removed.raw();
                    if self.lives.contains_key(&target) { *self.hero_damage.entry(*player).or_default() += removed.raw(); }
                }
            }
        }
        if let Some(mut trace) = world.try_fetch_mut::<omoba_core::runtime::native::damage_trace::DamageTrace>() {
            use omoba_core::runtime::native::damage_trace::DamageTraceResult;
            for row in trace.0.drain(..) {
                let Some(player) = self.lives.get(&row.source).or_else(|| self.owned.get(&row.source)).copied() else { continue };
                let ability = if row.ability.is_empty() { "(untagged)".to_string() } else { row.ability };
                let entry = self.ability_damage.entry((player, ability)).or_default();
                match row.result {
                    DamageTraceResult::Applied => {
                        let (physical, magical, pure) = row.after.unwrap_or((omoba_sim::Fixed64::ZERO, omoba_sim::Fixed64::ZERO, omoba_sim::Fixed64::ZERO));
                        let total = physical.raw() + magical.raw() + pure.raw();
                        entry[0] += total;
                        if self.lives.contains_key(&row.target) { entry[1] += total; }
                    }
                    DamageTraceResult::Cancelled => entry[2] += 1,
                    DamageTraceResult::SkippedSourceDead => entry[3] += 1,
                }
            }
        }
        for fact in facts {
            match &fact.fact {
                ObservableFact::Ability { source, ability_id, .. } => {
                    if let Some(player) = owners.get(source) { *self.accepted.entry((*player, *ability_id)).or_default() += 1; }
                }
                _ => {}
            }
        }
    }

    fn report(&self, world: &specs::World) -> serde_json::Value {
        use omoba_core::runtime::native::cast_trace::{CastTrace, CastTraceResult};
        let state = world.read_resource::<MobaMatch>();
        let mut results: std::collections::BTreeMap<(u32, String), std::collections::BTreeMap<String, u32>> = Default::default();
        if let Some(trace) = world.try_fetch::<CastTrace>() {
            for row in &trace.0 {
                // The life that cast (E318): earlier lives' rows count too.
                let Some(player) = self.lives.get(&row.caster).copied()
                    .or_else(|| state.heroes.iter().find(|slot| slot.entity == Some(row.caster)).map(|slot| slot.player_id)) else { continue };
                let result = match &row.result {
                    CastTraceResult::Accepted => "accepted".to_string(),
                    CastTraceResult::WindupStarted => "windup".to_string(),
                    CastTraceResult::Rejected(code) => format!("rejected:{code}"),
                    CastTraceResult::ScriptError { code, .. } => format!("script_error:{code}"),
                    CastTraceResult::Interrupted(reason) => format!("interrupted:{reason}"),
                    // The serial settlement refused an accepted cast (E373).
                    CastTraceResult::SettlementRefused(reason) => format!("settlement_refused:{reason}"),
                };
                *results.entry((player, row.ability.clone())).or_default().entry(result).or_default() += 1;
            }
        }
        serde_json::Value::Array(state.heroes.iter().map(|slot| {
            let (hero, _) = self.heroes.get(&slot.player_id).cloned().unwrap_or_default();
            // Every ability the hero held in a slot, swapped-out ones included (E400).
            let abilities = self.slots.iter().filter(|((player, _), _)| *player == slot.player_id)
                .map(|((_, ability), index)| (*index, ability.clone()));
            let casts: serde_json::Map<String, serde_json::Value> = abilities.map(|(index, ability)| {
                let stable = omoba_core::runtime::presentation_cue::stable_content_fact_id(&ability);
                (ability.clone(), serde_json::json!({
                    "slot": index,
                    "inputs": self.inputs.get(&(slot.player_id, ability.clone())).copied().unwrap_or(0),
                    "accepted": self.accepted.get(&(slot.player_id, stable)).copied().unwrap_or(0),
                    "results": results.get(&(slot.player_id, ability.clone())).cloned().unwrap_or_default(),
                }))
            }).collect();
            // Committed item uses publish an `item:<id>` cast fact (publish_item_use); obtained counts above.
            let items: serde_json::Map<String, serde_json::Value> = omoba_template_ids::MOBA_ITEM_CATALOG.iter()
                .filter(|item| item.in_shop())
                .filter_map(|item| {
                    let stable = omoba_core::runtime::presentation_cue::stable_content_fact_id(
                        &format!("{}{}", omoba_core::runtime::presentation_cue::ITEM_CAST_PREFIX, item.id));
                    let used = self.accepted.get(&(slot.player_id, stable)).copied().unwrap_or(0);
                    let obtained = self.obtained.get(&(slot.player_id, item.id.to_string())).copied().unwrap_or(0);
                    (used > 0 || obtained > 0).then(|| (item.id.to_string(), serde_json::json!({"obtained": obtained, "used": used})))
                }).collect();
            serde_json::json!({"player": slot.player_id, "hero": hero, "kills": slot.kills, "deaths": slot.deaths,
                "assists": slot.assists, "summons": self.summons.get(&slot.player_id).map_or(0, |s| s.len()), "casts": casts,
                "items": items, "team": state.config.teams.get(slot.side).copied().unwrap_or(0),
                "damage": omoba_sim::Fixed64::from_raw(self.damage.get(&slot.player_id).copied().unwrap_or(0)).to_f32_for_render().round() as i64,
                "hero_damage": omoba_sim::Fixed64::from_raw(self.hero_damage.get(&slot.player_id).copied().unwrap_or(0)).to_f32_for_render().round() as i64,
                "ability_damage": self.ability_damage.range((slot.player_id, String::new())..)
                    .take_while(|((player, _), _)| *player == slot.player_id)
                    .map(|((_, ability), [damage, hero, cancelled, skipped])| (ability.clone(), serde_json::json!({
                        "damage": omoba_sim::Fixed64::from_raw(*damage).to_f32_for_render().round() as i64,
                        "hero_damage": omoba_sim::Fixed64::from_raw(*hero).to_f32_for_render().round() as i64,
                        "cancelled": cancelled, "skipped_source_dead": skipped})))
                    .collect::<serde_json::Map<String, serde_json::Value>>(),
                "invalid_targets": self.invalid_targets.range((slot.player_id, String::new())..)
                    .take_while(|((player, _), _)| *player == slot.player_id)
                    .map(|((_, ability), rows)| (ability.clone(), serde_json::Value::from(rows.clone())))
                    .collect::<serde_json::Map<String, serde_json::Value>>()})
        }).collect())
    }
}
