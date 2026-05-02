//! Phase 2 lockstep wire integration test (placeholder for manual verify).
//!
//! The full happy-path harness — spinning up a real tokio KCP server,
//! connecting two `GameClient` instances, exchanging JoinRequest /
//! GameStart / InputSubmit / TickBatch — was deliberately deferred from
//! Phase 2.5. Spinning an in-process server here would have to:
//!
//! 1. Construct a fresh ECS World + dispatcher (omb's main.rs assembles ~30
//!    resources/systems before the transport thread starts), or
//! 2. Spawn `target/debug/omobab.exe` as a subprocess + parse stdout for
//!    "listening on …" before dialing.
//!
//! Both paths add real test-infra surface (port allocation, child-process
//! lifecycle on Windows, DLL staging for `scripts/base_content.dll`,
//! game.toml STORY override) that pays off only when Phase 3 brings in
//! real client-side sim consumption. Until then the wire layer is
//! exercised by:
//!
//!   - `omb/src/lockstep/input_buffer.rs` — submit + drain + evict tests
//!   - `omb/src/lockstep/tick_broadcaster.rs` — fire_one_tick / hash /
//!     channel-closed / 60-tick eviction tests (Phase 2.5)
//!   - `omb/src/transport/kcp_transport.rs` tests — frame encoding,
//!     InputSubmit decode roundtrip, JoinRole mapping
//!
//! The two `#[ignore]` placeholders below pin the *shape* of the network
//! roundtrip test that Phase 3 will fill in, and document the required
//! invariants in the function bodies as comments.
//!
//! Run with: `cargo test --test lockstep_integration -- --ignored`

#[tokio::test]
#[ignore] // requires real omb server; run via run.bat manually for Phase 2
async fn two_clients_receive_synchronized_tick_batch() {
    // Required invariants (Phase 3 will assert these):
    //   1. Both `GameClient::connect("127.0.0.1:50062").await` succeed.
    //   2. Each calls `join_lockstep(name, JoinRole::Player)` and receives
    //      a `GameStart { player_id, master_seed, start_tick }`. The two
    //      player_ids differ, the two master_seeds match.
    //   3. Each subscribes via `subscribe_lockstep()` returning a stream
    //      of `LockstepFrame`.
    //   4. Each calls `submit_input(target_tick = start_tick + 10,
    //      PlayerInput::NoOp)`.
    //   5. Both streams emit `TickBatch { tick: start_tick + 10, inputs }`
    //      where `inputs.len() == 2` and `inputs.iter().map(|i| i.player_id)
    //      .collect::<BTreeSet<_>>()` covers both player_ids.
    //   6. Both clients see TickBatches in identical order (deterministic
    //      replay precondition for omoba-sim consumption in Phase 3).
    panic!(
        "Phase 2.5 placeholder: spin up server + 2-client roundtrip is \
         deferred to Phase 3 when omoba-sim worker thread lands on omfx. \
         Manual smoke: run.bat + observe legacy GameEvent stream."
    );
}

#[tokio::test]
#[ignore]
async fn state_hash_broadcast_every_600_ticks() {
    // Required invariants:
    //   1. After GameStart, subscribe_lockstep stream receives the first
    //      StateHash at `tick == start_tick + 600` (10 seconds at 60Hz).
    //   2. Both clients receive the same StateHash (placeholder hash
    //      formula `tick * 0x9E3779B97F4A7C15` is purely tick-dependent
    //      so this is trivially true in Phase 2).
    //   3. Phase 3 swap to real ECS hash: clients running identical
    //      omoba-sim instances on identical TickBatch sequences must see
    //      bit-identical hashes — any divergence is a desync bug.
    panic!(
        "Phase 2.5 placeholder: see comment + tick_broadcaster unit tests \
         (`emits_state_hash_at_interval_multiples`, \
         `placeholder_state_hash_is_deterministic_pin`)."
    );
}
