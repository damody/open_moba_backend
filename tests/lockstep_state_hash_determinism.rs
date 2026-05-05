//! Phase 3.5 determinism test: two independent omobab Worlds initialized
//! with the same MasterSeed and ticked with identical input sequences must
//! produce bit-identical state hashes.
//!
//! This is the most important Phase 3 invariant: the server's
//! `compute_state_hash` and a client running the same simulation must
//! produce identical hashes. We can't easily run a real 8-client KCP
//! roundtrip in a unit test, but we can verify that running the same
//! input sequence on two independent World instances yields the same hash.
//!
//! # Running
//!
//! Marked `#[ignore]` because it requires the prebuilt
//! `scripts/target/release/base_content.dll` (skips if missing) and is
//! slow (loads campaign + scripts twice and runs 60 dispatcher ticks).
//!
//! ```text
//! # 1. Build the script DLL
//! cargo build --manifest-path scripts/Cargo.toml -p base_content --release
//!
//! # 2. Run the test
//! cargo test --manifest-path omb/Cargo.toml --test lockstep_state_hash_determinism \
//!     -- --ignored --nocapture
//! ```
//!
//! # What it verifies
//!
//! 1. `create_world_for_scene(TD_1)` x2 yields two worlds whose
//!    `compute_state_hash` matches at tick 0.
//! 2. Running `build_phase3_dispatcher` 60 times on each world (with empty
//!    PendingPlayerInputs) keeps hashes byte-identical at every tick.
//! 3. If a hash diverges, the test reports the exact tick where divergence
//!    started — that's a Phase 4 determinism bug to investigate.
//!
//! Phase 3 only hashes `Pos.x.raw + Pos.y.raw + Hp.raw` (see
//! `state_hash_producer::HashItem`); creep waves + tower attacks + projectile
//! motion are the deterministic forcing functions in this idle-input
//! scenario. If they pass, the foundation is sound.

use std::path::PathBuf;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn dll_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("omb has a parent dir (omoba root)")
        .join("scripts/target/release/base_content.dll")
}

fn dll_dir() -> Option<PathBuf> {
    let primary = dll_path();
    if primary.exists() {
        return Some(primary.parent().unwrap().to_path_buf());
    }
    // Fallback: omb-staged copy used by run.bat.
    let staged = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/base_content.dll");
    if staged.exists() {
        return Some(staged.parent().unwrap().to_path_buf());
    }
    None
}

#[test]
#[ignore]
fn two_worlds_same_seed_same_hashes() -> TestResult {
    // Use TD_1 — simpler than MVP_1, has deterministic creep waves running
    // even without any player input.
    let scene = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("omb must live under the monorepo root")
        .join("scripts/lua_data/TD_1");

    let dir = match dll_dir() {
        Some(d) => d,
        None => {
            eprintln!(
                "Skipping test: base_content.dll not built. Run \
                 `cargo build -p base_content --release` from scripts/ first."
            );
            return Ok(());
        }
    };
    eprintln!("[determinism] using script dir: {}", dir.display());

    let master_seed: u64 = 0xDEAD_BEEF_CAFE_BABE;

    let mut world_a = omobab::state::initialization::create_world_for_scene(&scene)
        .map_err(|e| format!("create_world_for_scene(world_a) failed: {}", e))?;
    let mut world_b = omobab::state::initialization::create_world_for_scene(&scene)
        .map_err(|e| format!("create_world_for_scene(world_b) failed: {}", e))?;

    // Override MasterSeed in both worlds. create_world_for_scene installs
    // the default (0xDEAD_BEEF_CAFE_BABE) but we want this explicit so a
    // future default-change doesn't silently weaken the test.
    use specs::WorldExt;
    world_a.write_resource::<omobab::comp::resources::MasterSeed>().0 = master_seed;
    world_b.write_resource::<omobab::comp::resources::MasterSeed>().0 = master_seed;

    // Each world gets its own ScriptRegistry (load_scripts_dir is non-pure
    // — opens DLLs into abi_stable handles — but the loaded Manifest_Refs
    // are deterministic and the scripts themselves are stateless given the
    // same MasterSeed).
    let registry_a = omobab::scripting::loader::load_scripts_dir(&dir);
    let registry_b = omobab::scripting::loader::load_scripts_dir(&dir);
    world_a.insert(registry_a);
    world_b.insert(registry_b);

    let mut dispatcher_a = omobab::state::system_dispatcher::build_phase3_dispatcher()
        .map_err(|e| format!("build_phase3_dispatcher(a) failed: {}", e))?;
    let mut dispatcher_b = omobab::state::system_dispatcher::build_phase3_dispatcher()
        .map_err(|e| format!("build_phase3_dispatcher(b) failed: {}", e))?;

    // Tick 0 baseline.
    let h0_a = omobab::lockstep::compute_state_hash(&world_a);
    let h0_b = omobab::lockstep::compute_state_hash(&world_b);
    assert_eq!(
        h0_a, h0_b,
        "tick 0 baseline hash mismatch! world_a=0x{:016x} world_b=0x{:016x}",
        h0_a, h0_b
    );
    eprintln!("[determinism] tick=0 hash=0x{:016x} (worlds match)", h0_a);

    // Run 60 ticks. Both worlds receive the same (empty) input batch via
    // the PendingPlayerInputs resource which is already inserted by
    // create_world_for_scene → setup_campaign_ecs_world.
    use omobab::comp::resources::Tick;

    for tick in 1..=60u32 {
        // Empty PendingPlayerInputs (creep waves run regardless).
        // (PendingPlayerInputs is drained by player_input_tick at the start
        // of each dispatch — by leaving it empty here, both worlds see the
        // exact same empty batch.)

        dispatcher_a.dispatch(&world_a);
        world_a.maintain();
        world_a.write_resource::<Tick>().0 = tick as u64;

        dispatcher_b.dispatch(&world_b);
        world_b.maintain();
        world_b.write_resource::<Tick>().0 = tick as u64;

        let hash_a = omobab::lockstep::compute_state_hash(&world_a);
        let hash_b = omobab::lockstep::compute_state_hash(&world_b);

        assert_eq!(
            hash_a, hash_b,
            "tick {}: hash mismatch! world_a=0x{:016x} world_b=0x{:016x}",
            tick, hash_a, hash_b
        );
        if tick % 10 == 0 {
            eprintln!("[determinism] tick={:>2} hash=0x{:016x}", tick, hash_a);
        }
    }

    eprintln!("[determinism] PASS: 60 ticks, hashes match end-to-end");
    Ok(())
}
