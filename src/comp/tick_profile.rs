use std::collections::BTreeMap;

#[derive(Default)]
pub struct TickProfile {
    pub tick_count: u64,
    pub run_systems_ns: u128,
    pub script_dispatch_ns: u128,
    pub process_outcomes_ns: u128,
    pub variant_stats: BTreeMap<&'static str, VariantStat>,
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

        log::info!(
            "tick_profile window={} avg(ms) run_systems={:.3} script_dispatch={:.3} process_outcomes={:.3} (out={:.0}%)",
            Self::WINDOW,
            run_avg_ms,
            dispatch_avg_ms,
            outcomes_avg_ms,
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
    }

    fn reset_window(&mut self) {
        self.run_systems_ns = 0;
        self.script_dispatch_ns = 0;
        self.process_outcomes_ns = 0;
        self.variant_stats.clear();
    }
}
