use omoba_core::runtime::{RuntimeBroadcast, RuntimeEvent};

use crate::transport::OutboundMsg;

#[cfg(any(feature = "grpc", feature = "kcp"))]
use crate::transport::BroadcastPolicy;

pub fn runtime_event_to_outbound(event: RuntimeEvent) -> OutboundMsg {
    let RuntimeEvent {
        topic,
        kind,
        action,
        data,
        entity_pos,
        broadcast,
    } = event;

    let mut msg = if let Some((x, y)) = entity_pos {
        OutboundMsg::new_s_at(&topic, &kind, &action, data, x, y)
    } else {
        OutboundMsg::new_s(&topic, &kind, &action, data)
    };

    #[cfg(any(feature = "grpc", feature = "kcp"))]
    if let Some(policy) = broadcast {
        msg = msg.with_policy(runtime_broadcast_to_policy(policy));
    }

    msg
}

pub fn runtime_events_to_outbound(
    events: impl IntoIterator<Item = RuntimeEvent>,
) -> Vec<OutboundMsg> {
    events.into_iter().map(runtime_event_to_outbound).collect()
}

#[cfg(any(feature = "grpc", feature = "kcp"))]
pub fn runtime_broadcast_to_policy(policy: RuntimeBroadcast) -> BroadcastPolicy {
    match policy {
        RuntimeBroadcast::All => BroadcastPolicy::All,
        RuntimeBroadcast::AoiPoint(x, y) => BroadcastPolicy::AoiPoint(x, y),
        RuntimeBroadcast::AoiEntity(entity_id) => BroadcastPolicy::AoiEntity(entity_id),
        RuntimeBroadcast::PlayerOnly(player) => BroadcastPolicy::PlayerOnly(player),
    }
}
