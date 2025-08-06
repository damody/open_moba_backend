/// 事件分派器 - 統一管理所有遊戲事件的處理

use specs::{Entity, World};
use crate::comp::*;
use crate::msg::MqttMsg;
use crossbeam_channel::Sender;

use super::{
    CombatEventHandler, MovementEventHandler, 
    CreationEventHandler, SystemEventHandler
};

/// 事件分派器
pub struct EventDispatcher;

impl EventDispatcher {
    /// 處理單個遊戲結果事件
    pub fn dispatch_outcome(
        world: &World,
        mqtx: &Sender<MqttMsg>,
        outcome: Outcome,
    ) -> Vec<Outcome> {
        match outcome {
            // 戰鬥相關事件
            Outcome::Damage { pos, phys, magi, real, source, target } => {
                CombatEventHandler::handle_damage(world, mqtx, pos, phys, magi, real, source, target)
            }
            Outcome::Heal { pos, target, amount } => {
                CombatEventHandler::handle_heal(world, mqtx, pos, target, amount)
            }
            Outcome::Death { pos, ent } => {
                CombatEventHandler::handle_death(world, mqtx, pos, ent)
            }
            Outcome::GainExperience { target, amount } => {
                CombatEventHandler::handle_experience_gain(world, mqtx, target, amount)
            }
            Outcome::UpdateAttack { target, asd_count, cooldown_reset } => {
                CombatEventHandler::handle_attack_update(world, mqtx, target, asd_count, cooldown_reset)
            }

            // 移動相關事件
            Outcome::CreepStop { source, target } => {
                MovementEventHandler::handle_creep_stop(world, mqtx, source, target)
            }
            Outcome::CreepWalk { target } => {
                MovementEventHandler::handle_creep_walk(world, mqtx, target)
            }

            // 創建相關事件
            Outcome::Creep { cd } => {
                CreationEventHandler::handle_creep_creation(world, mqtx, cd)
            }
            Outcome::Tower { pos, td } => {
                CreationEventHandler::handle_tower_creation(world, mqtx, pos, td)
            }
            Outcome::ProjectileLine2 { pos, source, target } => {
                CreationEventHandler::handle_projectile_creation(world, mqtx, pos, source, target)
            }

            // 系統事件
            _ => {
                SystemEventHandler::handle_generic_event(world, mqtx, outcome)
            }
        }
    }

    /// 批量處理多個事件
    pub fn dispatch_outcomes_batch(
        world: &World,
        mqtx: &Sender<MqttMsg>,
        outcomes: Vec<Outcome>,
    ) -> Vec<Outcome> {
        let mut all_next_outcomes = Vec::new();
        
        for outcome in outcomes {
            let mut next_outcomes = Self::dispatch_outcome(world, mqtx, outcome);
            all_next_outcomes.append(&mut next_outcomes);
        }
        
        all_next_outcomes
    }

    /// 按優先級處理事件
    pub fn dispatch_outcomes_prioritized(
        world: &World,
        mqtx: &Sender<MqttMsg>,
        outcomes: Vec<Outcome>,
    ) -> Vec<Outcome> {
        // 按優先級排序事件
        let mut prioritized = Self::sort_outcomes_by_priority(outcomes);
        let mut all_next_outcomes = Vec::new();
        
        // 分批處理不同優先級的事件
        let high_priority: Vec<_> = prioritized.drain_filter(|o| Self::is_high_priority(o)).collect();
        let medium_priority: Vec<_> = prioritized.drain_filter(|o| Self::is_medium_priority(o)).collect();
        let low_priority = prioritized; // 剩下的都是低優先級
        
        // 先處理高優先級事件
        for outcome in high_priority {
            let mut next = Self::dispatch_outcome(world, mqtx, outcome);
            all_next_outcomes.append(&mut next);
        }
        
        // 再處理中優先級事件
        for outcome in medium_priority {
            let mut next = Self::dispatch_outcome(world, mqtx, outcome);
            all_next_outcomes.append(&mut next);
        }
        
        // 最後處理低優先級事件
        for outcome in low_priority {
            let mut next = Self::dispatch_outcome(world, mqtx, outcome);
            all_next_outcomes.append(&mut next);
        }
        
        all_next_outcomes
    }

    /// 統計事件類型分佈
    pub fn analyze_outcomes(outcomes: &[Outcome]) -> OutcomeAnalysis {
        let mut analysis = OutcomeAnalysis::default();
        
        for outcome in outcomes {
            match outcome {
                Outcome::Damage { .. } => analysis.combat_events += 1,
                Outcome::Heal { .. } => analysis.combat_events += 1,
                Outcome::Death { .. } => analysis.combat_events += 1,
                Outcome::GainExperience { .. } => analysis.combat_events += 1,
                Outcome::UpdateAttack { .. } => analysis.combat_events += 1,
                
                Outcome::CreepStop { .. } => analysis.movement_events += 1,
                Outcome::CreepWalk { .. } => analysis.movement_events += 1,
                
                Outcome::Creep { .. } => analysis.creation_events += 1,
                Outcome::Tower { .. } => analysis.creation_events += 1,
                Outcome::ProjectileLine2 { .. } => analysis.creation_events += 1,
                
                _ => analysis.system_events += 1,
            }
            analysis.total_events += 1;
        }
        
        analysis
    }

    // 私有輔助方法
    fn sort_outcomes_by_priority(mut outcomes: Vec<Outcome>) -> Vec<Outcome> {
        outcomes.sort_by(|a, b| {
            let priority_a = Self::get_outcome_priority(a);
            let priority_b = Self::get_outcome_priority(b);
            priority_b.cmp(&priority_a) // 高優先級先處理
        });
        outcomes
    }

    fn get_outcome_priority(outcome: &Outcome) -> u8 {
        match outcome {
            Outcome::Death { .. } => 10,           // 最高優先級
            Outcome::Damage { .. } => 8,           // 高優先級
            Outcome::Heal { .. } => 7,             // 高優先級
            Outcome::CreepStop { .. } => 6,        // 中高優先級
            Outcome::CreepWalk { .. } => 5,        // 中優先級
            Outcome::UpdateAttack { .. } => 4,     // 中優先級
            Outcome::ProjectileLine2 { .. } => 3,  // 中低優先級
            Outcome::Tower { .. } => 2,            // 低優先級
            Outcome::Creep { .. } => 2,            // 低優先級
            Outcome::GainExperience { .. } => 1,   // 最低優先級
            _ => 0,                                 // 系統事件
        }
    }

    fn is_high_priority(outcome: &Outcome) -> bool {
        Self::get_outcome_priority(outcome) >= 7
    }

    fn is_medium_priority(outcome: &Outcome) -> bool {
        let priority = Self::get_outcome_priority(outcome);
        priority >= 3 && priority < 7
    }
}

/// 事件分析結果
#[derive(Debug, Default, Clone)]
pub struct OutcomeAnalysis {
    pub total_events: usize,
    pub combat_events: usize,
    pub movement_events: usize,
    pub creation_events: usize,
    pub system_events: usize,
}

impl OutcomeAnalysis {
    /// 獲取各類事件的百分比
    pub fn get_percentages(&self) -> OutcomePercentages {
        if self.total_events == 0 {
            return OutcomePercentages::default();
        }
        
        let total = self.total_events as f32;
        OutcomePercentages {
            combat_percent: (self.combat_events as f32 / total) * 100.0,
            movement_percent: (self.movement_events as f32 / total) * 100.0,
            creation_percent: (self.creation_events as f32 / total) * 100.0,
            system_percent: (self.system_events as f32 / total) * 100.0,
        }
    }

    /// 檢查是否有性能問題
    pub fn check_performance_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        
        if self.total_events > 1000 {
            issues.push("事件數量過多，可能影響性能".to_string());
        }
        
        if self.combat_events > self.total_events / 2 {
            issues.push("戰鬥事件過多，考慮批量處理".to_string());
        }
        
        if self.creation_events > 100 {
            issues.push("創建事件過多，考慮對象池".to_string());
        }
        
        issues
    }
}

/// 事件百分比統計
#[derive(Debug, Default, Clone)]
pub struct OutcomePercentages {
    pub combat_percent: f32,
    pub movement_percent: f32,
    pub creation_percent: f32,
    pub system_percent: f32,
}

// 臨時的 drain_filter 實現，因為它還不穩定
trait DrainFilterExt<T> {
    fn drain_filter<F>(&mut self, f: F) -> Vec<T>
    where
        F: FnMut(&T) -> bool;
}

impl<T> DrainFilterExt<T> for Vec<T> {
    fn drain_filter<F>(&mut self, mut f: F) -> Vec<T>
    where
        F: FnMut(&T) -> bool,
    {
        let mut i = 0;
        let mut result = Vec::new();
        
        while i < self.len() {
            if f(&self[i]) {
                result.push(self.remove(i));
            } else {
                i += 1;
            }
        }
        
        result
    }
}