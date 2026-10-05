//! Single-lane acceptance executable: real script DLL, formal inputs, replay.
use std::path::{Path, PathBuf};

use failure::{err_msg, Error};
use omoba_core::runtime::*;
use serde_json::json;
use specs::WorldExt;
use omoba_core::runtime::native::moba_match::bots::{role_bot_inputs, RoleBotMatchPlan};

fn make_world(config: SingleLaneConfig, scripts_dir: &Path) -> Result<specs::World, Error> {
    let scripts = omoba_core::scripting::loader::load_scripts_dir(scripts_dir);
    create_single_lane_world(config, scripts)
}

fn run() -> Result<(), Error> {
    let mut scripts_dir = PathBuf::from("scripts/target/release");
    let mut report_path = PathBuf::from("omb/target/moba-headless/report.json");
    let mut config = SingleLaneConfig::default();
    let mut profile = SimulationTickProfile::Production60Hz;
    let mut withdraw_defender = false;
    let mut role_plan_path = None;
    let mut plan_only = false;
    let mut fixture_options = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| err_msg(format!("{arg} requires a value")))?;
        match arg.as_str() {
            "--scripts-dir" => scripts_dir = PathBuf::from(value),
            "--report" => report_path = PathBuf::from(value),
            "--role-plan" => {
                if role_plan_path.is_some() { return Err(err_msg("duplicate role-plan option")); }
                role_plan_path = Some(PathBuf::from(value));
            },
            "--plan-only" => plan_only = value.parse().map_err(|_| err_msg("plan-only must be true or false"))?,
            "--seed" => config.seed = value.parse().map_err(|_| err_msg("invalid seed"))?,
            "--map" => {
                fixture_options = true;
                let map = omoba_template_ids::moba_map_by_name(&value)
                    .ok_or_else(|| err_msg(format!("unknown compiled MOBA map: {value}")))?;
                config.lane_length = omoba_sim::Fixed64::from_i32(map.lane_length);
                config.map_id = Some(value);
            }
            "--defender" => { fixture_options = true; withdraw_defender = match value.as_str() {
                "guard" => false,
                "withdraw" => true,
                _ => return Err(err_msg("defender must be guard or withdraw")),
            }; },
            "--profile" => {
                profile = match value.as_str() {
                    "15" => SimulationTickProfile::Coarse15Hz,
                    "60" => SimulationTickProfile::Production60Hz,
                    "120" => SimulationTickProfile::Production120Hz,
                    _ => return Err(err_msg("profile must be 15, 60 or 120")),
                }
            }
            _ => return Err(err_msg(format!("unknown argument: {arg}"))),
        }
    }
    if role_plan_path.is_some() && fixture_options {
        return Err(err_msg("role-plan cannot be combined with fixture map/defender options"));
    }
    let role_plan = role_plan_path.as_ref().map(|path| -> Result<RoleBotMatchPlan, Error> {
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }).transpose()?;
    let bots = if let Some(plan) = &role_plan {
        let (planned, bots) = plan.compile(config.seed, profile).map_err(err_msg)?;
        config = planned;
        Some(bots)
    } else { None };
    if plan_only {
        let plan = role_plan.as_ref().ok_or_else(|| err_msg("plan-only requires role-plan"))?;
        let bots = bots.as_ref().expect("compiled plan");
        let report = json!({"success":true,"scope":"configuration only; no simulation or match acceptance",
            "profile_hz":profile.ticks_per_game_second(),"plan":plan,
            "bot_player_ids":bots.assignments.iter().map(|a|a.player_id).collect::<Vec<_>>(),
            "think_interval_ticks":bots.think_interval_ticks});
        if let Some(parent) = report_path.parent() { std::fs::create_dir_all(parent)?; }
        std::fs::write(&report_path,serde_json::to_vec_pretty(&report)?)?;
        println!("MOBA role plan valid: {} players, {} bots",plan.players.len(),bots.assignments.len());
        return Ok(());
    }
    let mut world = make_world(config.clone(), &scripts_dir)?;
    if let Some(bots) = &bots { bots.validate(&world.read_resource::<MobaMatch>()).map_err(err_msg)?; }
    let mut driver = SimulationDriver::from_world(&mut world, profile)?;
    let max_ticks = u64::from(profile.ticks_per_game_second()) * 600;
    let mut inputs_recorded = Vec::new();
    let mut digests = Vec::new();
    let mut casts = [0u32; 4];
    let mut end_events = 0;
    let mut combat_facts = 0;
    for _ in 0..max_ticks {
        let mut inputs = if let Some(bots) = &bots { role_bot_inputs(&world,bots).map_err(err_msg)? } else { single_lane_bot_inputs(
            &world,
            [SingleLaneBotPolicy::Push, SingleLaneBotPolicy::Guard],
        ) };
        if withdraw_defender && world.read_resource::<MobaMatch>().phase == MobaMatchPhase::Playing {
            // A noncompetitive completion fixture using only ordinary inputs.
            // No health, tower, progression or phase injection.
            inputs.retain(|(player,_)| *player != config.players[1]);
            inputs.push((config.players[1],PlayerInput { action: Some(PlayerInputEnum::MoveTo(MoveTo {
                target: Some(Vec2I { x: config.lane_length.raw() as i32,y:3000*1024 }),queued:false,
            })) }));
        }
        for (_, input) in &inputs {
            if let Some(PlayerInputEnum::CastAbility(cast)) = &input.action {
                if let Some(count) = casts.get_mut(cast.ability_index as usize) {
                    *count += 1;
                }
            }
        }
        let result = driver.step(&mut world, inputs.clone())?;
        if bots.is_some() { run_committed_visibility_wave_b(&mut world,result.tick,0); }
        combat_facts += result
            .facts
            .iter()
            .filter(|f| matches!(f.fact, ObservableFact::DirectCombat { .. }))
            .count();
        end_events += result
            .events
            .iter()
            .filter(|e| e.topic == "game.end")
            .count();
        inputs_recorded.push(inputs);
        digests.push(single_lane_replay_digest(&world));
        if matches!(
            world.read_resource::<MobaMatch>().phase,
            MobaMatchPhase::Finished { .. }
        ) {
            break;
        }
    }
    let state = world.read_resource::<MobaMatch>().clone();
    let (winner, finish_tick) = match state.phase {
        MobaMatchPhase::Finished { winner, tick } => (winner, tick),
        phase => {
            return Err(err_msg(format!(
                "match timeout after {max_ticks} ticks: {phase:?}"
            )))
        }
    };
    if end_events != 1 || (bots.is_none() && casts.contains(&0)) || combat_facts == 0 {
        return Err(err_msg(format!(
            "incomplete acceptance: end_events={end_events}, casts={casts:?}"
        )));
    }
    let mut replay = make_world(config.clone(), &scripts_dir)?;
    let mut replay_driver = SimulationDriver::from_world(&mut replay, profile)?;
    for (index, (inputs, expected)) in inputs_recorded.into_iter().zip(&digests).enumerate() {
        replay_driver.step(&mut replay, inputs)?;
        let actual = single_lane_replay_digest(&replay);
        if &actual != expected {
            return Err(err_msg(format!(
                "replay diverged at tick {}: {expected} != {actual}",
                index + 1
            )));
        }
    }
    let report = json!({
        "success": true, "profile_hz": profile.ticks_per_game_second(), "seed": config.seed,
        "role_plan":role_plan,
        "bot_mode":if bots.is_some() {"committed_role_plan"} else {"legacy_fixture"},
        "skill_coverage_checked":bots.is_none(),
        "map_id": config.map_id, "lane_count": state.lane_towers.len(),
        "defender_policy": if withdraw_defender { "withdraw" } else { "guard" },
        "remaining_lane_towers": state.lane_towers.iter().map(|lane|
            lane.map(|tower| tower.is_some())).collect::<Vec<_>>(),
        "script_dll": scripts_dir.join("base_content.dll"), "winner_team": winner,
        "finish_tick": finish_tick, "game_seconds": state.elapsed.raw() as f64 / 1024.0,
        "waves": state.waves, "deaths": state.heroes.iter().map(|h| h.deaths).collect::<Vec<_>>(),
        "respawns": state.heroes.iter().map(|h| h.respawns).collect::<Vec<_>>(),
        "cast_inputs_by_slot": casts, "end_events": end_events,
        "committed_combat_facts": combat_facts,
        "replay_verified_ticks": digests.len(), "final_digest": digests.last(),
        "tick_digests": digests,
        "scope": "authoritative headless ECS; not Unreal, filtered replica, LAN, or full game acceptance"
    });
    if let Some(parent) = report_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("MOBA headless PASS: winner={winner:?}, tick={finish_tick}, waves={}, casts={casts:?}, replay={} ticks; report={}",
        state.waves, report["replay_verified_ticks"], report_path.display());
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("MOBA headless FAILED: {error}");
        std::process::exit(1);
    }
}
