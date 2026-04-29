use std::collections::BTreeMap;

#[derive(Default)]
pub struct TickProfile {
    pub tick_count: u64,
    pub run_systems_ns: u128,
    pub script_dispatch_ns: u128,
    pub process_outcomes_ns: u128,
    pub variant_stats: BTreeMap<&'static str, VariantStat>,
    /// Per-script-id timing：script id（"dart" / "ice" / ...）→ (count, total_ns)。
    /// 在 dispatch.rs 的 on_tick 迴圈每次量測後 record。Window 結束時 emit_log 印
    /// top N 最耗時 scripts，可確認 script_dispatch_ns 主要花在哪。
    pub script_stats: BTreeMap<String, VariantStat>,
    /// 本 window 累積的 queued events 總耗時（Spawn / Death / AttackHit ... 之類）
    pub script_events_ns: u128,
    pub script_events_count: u64,
}

#[derive(Default, Clone, Copy)]
pub struct VariantStat {
    pub count: u64,
    pub ns: u128,
}

#[derive(Copy, Clone)]
pub enum Phase {
    RunSystems,
    ScriptDispatch,
    ProcessOutcomes,
}

impl TickProfile {
    pub const WINDOW: u64 = 60;

    pub fn record_phase(&mut self, phase: Phase, ns: u128) {
        match phase {
            Phase::RunSystems => self.run_systems_ns += ns,
            Phase::ScriptDispatch => self.script_dispatch_ns += ns,
            Phase::ProcessOutcomes => self.process_outcomes_ns += ns,
        }
    }

    pub fn record_variant(&mut self, kind: &'static str, ns: u128) {
        let entry = self.variant_stats.entry(kind).or_default();
        entry.count += 1;
        entry.ns += ns;
    }

    /// 記錄一次 UnitScript on_tick 的耗時。`script_id` 對應 manifest 的 unit_id。
    pub fn record_script(&mut self, script_id: &str, ns: u128) {
        let entry = self.script_stats.entry(script_id.to_string()).or_default();
        entry.count += 1;
        entry.ns += ns;
    }

    /// 記錄一次 queued event dispatch（Spawn / AttackHit / Death / ...）的耗時。
    pub fn record_script_event(&mut self, ns: u128) {
        self.script_events_ns += ns;
        self.script_events_count += 1;
    }

    pub fn finish_tick_and_maybe_log(&mut self) {
        self.tick_count += 1;
        if self.tick_count % Self::WINDOW == 0 {
            self.emit_log();
            self.reset_window();
        }
    }

    fn emit_log(&self) {
        let window = Self::WINDOW as f64;
        // ns → ms: ÷ 1_000_000. window 個 tick 取平均。
        let run_avg_ms = self.run_systems_ns as f64 / window / 1_000_000.0;
        let dispatch_avg_ms = self.script_dispatch_ns as f64 / window / 1_000_000.0;
        let outcomes_avg_ms = self.process_outcomes_ns as f64 / window / 1_000_000.0;
        let total_avg_ms = run_avg_ms + dispatch_avg_ms + outcomes_avg_ms;

        let outcomes_pct = if total_avg_ms > 0.0 {
            (outcomes_avg_ms * 100.0) / total_avg_ms
        } else {
            0.0
        };

        // total_ms = 一個 tick 的實際執行時間（run + dispatch + outcomes）
        // max_tps = 1秒 (1000ms) ÷ total_ms = 理論可達 tick/sec 上限
        // （實際 server clock 限制在 main.rs 的 TPS = 30；max_tps 越高代表
        // 還有多少餘裕。例如 max_tps=660 表示目前模擬負擔 << 30 TPS budget）
        let max_tps = if total_avg_ms > 0.0 {
            (1000.0 / total_avg_ms) as u32
        } else {
            u32::MAX
        };

        log::info!(
            "tick_profile window={} avg(ms) run={:.3} dispatch={:.3} outcomes={:.3} total={:.3} (max_tps={}, out={:.0}%)",
            Self::WINDOW,
            run_avg_ms,
            dispatch_avg_ms,
            outcomes_avg_ms,
            total_avg_ms,
            max_tps,
            outcomes_pct,
        );

        let mut entries: Vec<_> = self.variant_stats.iter().collect();
        entries.sort_by(|a, b| b.1.ns.cmp(&a.1.ns));
        for (name, stat) in entries.iter().take(6) {
            let total_ms = stat.ns as f64 / 1_000_000.0;
            let avg_ms = if stat.count > 0 {
                stat.ns as f64 / stat.count as f64 / 1_000_000.0
            } else {
                0.0
            };
            log::info!(
                "  outcome {:<22} count={:>6} total_ms={:>7.3} avg_ms={:>7.4}",
                name,
                stat.count,
                total_ms,
                avg_ms,
            );
        }

        // Script dispatch 細項：on_tick 各 script id 排序 + queued events 總和
        if !self.script_stats.is_empty() || self.script_events_count > 0 {
            let events_total_ms = self.script_events_ns as f64 / 1_000_000.0;
            let events_avg_us = if self.script_events_count > 0 {
                self.script_events_ns as f64 / self.script_events_count as f64 / 1_000.0
            } else {
                0.0
            };
            log::info!(
                "  script  events                count={:>6} total_ms={:>7.3} avg_us={:>7.2}",
                self.script_events_count,
                events_total_ms,
                events_avg_us,
            );
            let mut s_entries: Vec<_> = self.script_stats.iter().collect();
            s_entries.sort_by(|a, b| b.1.ns.cmp(&a.1.ns));
            for (name, stat) in s_entries.iter().take(8) {
                let total_ms = stat.ns as f64 / 1_000_000.0;
                let avg_us = if stat.count > 0 {
                    stat.ns as f64 / stat.count as f64 / 1_000.0
                } else {
                    0.0
                };
                log::info!(
                    "  script  {:<22} count={:>6} total_ms={:>7.3} avg_us={:>7.2}",
                    name,
                    stat.count,
                    total_ms,
                    avg_us,
                );
            }
        }
    }

    fn reset_window(&mut self) {
        self.run_systems_ns = 0;
        self.script_dispatch_ns = 0;
        self.process_outcomes_ns = 0;
        self.variant_stats.clear();
        self.script_stats.clear();
        self.script_events_ns = 0;
        self.script_events_count = 0;
    }
}
