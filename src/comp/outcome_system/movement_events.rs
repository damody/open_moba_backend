/// 移動相關事件處理

use specs::{Entity, World};
use crate::comp::*;
use crate::msg::MqttMsg;
use crossbeam_channel::Sender;

/// 移動事件處理器
pub struct MovementEventHandler;

impl MovementEventHandler {
    /// 處理小兵停止事件
    pub fn handle_creep_stop(
        _world: &World,
        _mqtx: &Sender<MqttMsg>,
        _source: Entity,
        _target: Entity,
    ) -> Vec<Outcome> {
        // 實現小兵停止邏輯
        Vec::new()
    }

    /// 處理小兵行走事件
    pub fn handle_creep_walk(
        _world: &World,
        _mqtx: &Sender<MqttMsg>,
        _target: Entity,
    ) -> Vec<Outcome> {
        // 實現小兵行走邏輯
        Vec::new()
    }
}