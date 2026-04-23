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
        let window = Self::WINDOW as u128;
        let run_avg_us = self.run_systems_ns / window / 1_000;
        let dispatch_avg_us = self.script_dispatch_ns / window / 1_000;
        let outcomes_avg_us = self.process_outcomes_ns / window / 1_000;
        let total_avg_us = run_avg_us + dispatch_avg_us + outcomes_avg_us;

        let outcomes_pct = if total_avg_us > 0 {
            (outcomes_avg_us * 100) / total_avg_us
        } else {
            0
        };

        log::info!(
            "tick_profile window={} avg(μs) run_systems={} script_dispatch={} process_outcomes={} (out={}%)",
            Self::WINDOW,
            run_avg_us,
            dispatch_avg_us,
            outcomes_avg_us,
            outcomes_pct,
        );

        let mut entries: Vec<_> = self.variant_stats.iter().collect();
        entries.sort_by(|a, b| b.1.ns.cmp(&a.1.ns));
        for (name, stat) in entries.iter().take(6) {
            let avg_ns = if stat.count > 0 {
                stat.ns / stat.count as u128
            } else {
                0
            };
            log::info!(
                "  outcome {:<22} count={:>6} total_us={:>6} avg_ns={:>5}",
                name,
                stat.count,
                stat.ns / 1_000,
                avg_ns,
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
