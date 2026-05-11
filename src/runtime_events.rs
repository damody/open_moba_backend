use omoba_core::runtime::{RuntimeBroadcast, RuntimeEvent};
#[cfg(feature = "kcp")]
use serde_json::Value;

use crate::transport::OutboundMsg;

#[cfg(any(feature = "grpc", feature = "kcp"))]
use crate::transport::BroadcastPolicy;

pub fn runtime_event_to_outbound(event: RuntimeEvent) -> OutboundMsg {
    #[cfg(feature = "kcp")]
    if let Some(msg) = typed_runtime_event_to_outbound(&event) {
        return msg;
    }

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

#[cfg(feature = "kcp")]
fn typed_runtime_event_to_outbound(event: &RuntimeEvent) -> Option<OutboundMsg> {
    use crate::state::resource_management::proto_build;
    use crate::transport::TypedOutbound;

    match (
        event.topic.as_str(),
        event.kind.as_str(),
        event.action.as_str(),
    ) {
        ("td/all/res", "game", "lives") => {
            let lives = event.data.get("lives")?.as_i64()? as i32;
            Some(OutboundMsg::new_typed_all(
                "td/all/res",
                "game",
                "lives",
                TypedOutbound::GameLives(proto_build::game_lives(lives)),
                event.data.clone(),
            ))
        }
        ("td/all/res", "game", "end") => {
            let winner = game_end_winner(&event.data);
            Some(OutboundMsg::new_typed_all(
                "td/all/res",
                "game",
                "end",
                TypedOutbound::GameEnd(proto_build::game_end(&winner)),
                event.data.clone(),
            ))
        }
        _ => None,
    }
}

#[cfg(feature = "kcp")]
fn game_end_winner(data: &Value) -> String {
    data.get("winner")
        .or_else(|| data.get("result"))
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string()
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
