//! 階段 3.5 確定性測試：初始化兩個獨立的 omobab 世界
//! 必須使用相同的 MasterSeed 並勾選相同的輸入序列
//! 產生位元相同的狀態哈希。
//!
//! 這是最重要的第 3 階段不變量：伺服器的
//! `compute_state_hash` 和執行相同模擬的客戶端必須
//! 產生相同的哈希值。我們無法輕鬆運行真正的 8 客戶端 KCP
//! 單元測試中的往返，但我們可以驗證運行相同的
//! 兩個獨立 World 實例上的輸入序列產生相同的雜湊值。
//!
//! ＃ 跑步
//!
//! 標記為“#[ignore]”，因為它需要預先構建
//! `scripts/target/release/base_content.dll`（如果遺失則跳過）且是
//! 慢（載入活動 + 腳本兩次並執行 60 個調度程序滴答）。
//!
//! ````文本
//! # 1. 建置腳本DLL
//! 貨物建構 --manifest-path 腳本/Cargo.toml -p base_content --release
//!
//! # 2. 運行測試
//! 貨物測試 --manifest-path omb/Cargo.toml --test lockstep_state_hash_definism \
//! -- --忽略 --nocapture
//! ```
//!
//! # 它驗證什麼
//!
//! 1. `create_world_for_scene(TD_1)` x2 產生兩個世界，其
//! `compute_state_hash` 在刻度 0 處符合。
//! 2. 在每個世界運行 `build_phase3_dispatcher` 60 次（空
//! PendingPlayerInputs) 在每次更新時都保持雜湊位元組相同。
//! 3. 如果雜湊出現分歧，測試會報告出現分歧的確切刻度
//! 開始了——這是一個需要調查的第四階段決定論錯誤。
//!
//! 第 3 階段僅對 `Pos.x.raw + Pos.y.raw + Hp.raw` 進行雜湊處理（請參閱
//! `state_hash_ Producer::HashItem`);蠕動波+塔攻擊+彈
//! 運動是該空閒輸入中的確定性強迫函數
//! 設想。如果他們通過了，那麼基礎就牢固了。

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
    // 後備：run.bat 使用的 omb 階段副本。
    let staged = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/base_content.dll");
    if staged.exists() {
        return Some(staged.parent().unwrap().to_path_buf());
    }
    None
}

#[test]
#[ignore]
fn two_worlds_same_seed_same_hashes() -> TestResult {
    // 使用 TD_1 — 比 MVP_1 更簡單，具有確定性的蠕變波運行
    // 即使沒有任何玩家輸入。
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

    // 在兩個世界中覆蓋 MasterSeed。 create_world_for_scene 安裝
    // 預設值（0xDEAD_BEEF_CAFE_BABE），但我們希望它是明確的，因此
    // 未來的預設更改不會默默地削弱測試。
    use specs::WorldExt;
    world_a.write_resource::<omobab::comp::resources::MasterSeed>().0 = master_seed;
    world_b.write_resource::<omobab::comp::resources::MasterSeed>().0 = master_seed;

    // 每個世界都有自己的 ScriptRegistry （load_scripts_dir 是非純的
    // — 將 DLL 開啟到 abi_stable 句柄中 — 但載入的 Manifest_Refs
    // 是確定性的，並且腳本本身是無狀態的
    // 相同的 MasterSeed）。
    let registry_a = omobab::scripting::loader::load_scripts_dir(&dir);
    let registry_b = omobab::scripting::loader::load_scripts_dir(&dir);
    world_a.insert(registry_a);
    world_b.insert(registry_b);

    let mut dispatcher_a = omobab::state::system_dispatcher::build_phase3_dispatcher()
        .map_err(|e| format!("build_phase3_dispatcher(a) failed: {}", e))?;
    let mut dispatcher_b = omobab::state::system_dispatcher::build_phase3_dispatcher()
        .map_err(|e| format!("build_phase3_dispatcher(b) failed: {}", e))?;

    // 勾選 0 基線。
    let h0_a = omobab::lockstep::compute_state_hash(&world_a);
    let h0_b = omobab::lockstep::compute_state_hash(&world_b);
    assert_eq!(
        h0_a, h0_b,
        "tick 0 baseline hash mismatch! world_a=0x{:016x} world_b=0x{:016x}",
        h0_a, h0_b
    );
    eprintln!("[determinism] tick=0 hash=0x{:016x} (worlds match)", h0_a);

    // 運行 60 個刻度。兩個世界透過以下方式接收相同的（空）輸入批次
    // 已插入的 PendingPlayerInputs 資源
    // create_world_for_scene → setup_campaign_ecs_world。
    use omobab::comp::resources::Tick;

    for tick in 1..=60u32 {
        // 清空 PendingPlayerInputs（無論如何，蠕變波都會運作）。
        // （PendingPlayerInputs 在開始時由player_input_tick 耗盡
        // 每份派遣的訊息 — 透過將其留空，兩個世界都能看到
        // 完全相同的空白批次。 ）

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
