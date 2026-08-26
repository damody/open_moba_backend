use omoba_core::runtime::{
    FactAudience, FactKind, FactOrderingKey, FactPhase, ObservableFact, OrderedFact,
    OrderedOutput, RuntimeBroadcast, RuntimeEvent,
};
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

pub fn ordered_runtime_events_to_outbound(
    events: impl IntoIterator<Item = OrderedOutput<RuntimeEvent>>,
) -> Vec<OutboundMsg> {
    let mut events: Vec<_> = events.into_iter().collect();
    events.retain(|event| event.key.validate().is_ok());
    events.sort_by_key(|event| event.key);
    events.into_iter().map(|event| runtime_event_to_outbound(event.value)).collect()
}

/// Transitional adapter for legacy Outcome producers. The caller supplies the
/// authoritative tick/phase; vector position is canonical only after the
/// deterministic Outcome processor has completed.
pub fn order_processed_runtime_events(
    tick: u64,
    phase: FactPhase,
    events: Vec<RuntimeEvent>,
) -> Vec<OrderedOutput<RuntimeEvent>> {
    events
        .into_iter()
        .enumerate()
        .map(|(ordinal, value)| OrderedOutput {
            key: FactOrderingKey {
                tick,
                phase,
                canonical_source_order: ordinal as u64,
                local_ordinal: 0,
                fact_kind: runtime_event_fact_kind(&value),
            },
            value,
        })
        .collect()
}

fn runtime_event_fact_kind(event: &RuntimeEvent) -> FactKind {
    match (event.kind.as_str(), event.action.as_str()) {
        ("game", "end") => FactKind::Terminal,
        ("game", _) => FactKind::Hud,
        ("unit", "spawn") | ("creep", "spawn") | ("tower", "spawn") => FactKind::Spawn,
        ("unit", "death") | ("creep", "death") | ("tower", "death") => FactKind::Death,
        ("projectile", _) => FactKind::Projectile,
        ("buff", _) => FactKind::Buff,
        ("ability", _) => FactKind::Ability,
        ("item", _) => FactKind::Item,
        ("tower", _) => FactKind::Tower,
        ("move", _) | ("movement", _) => FactKind::Movement,
        _ => FactKind::DirectCombat,
    }
}

/// Retained global UI events cross the selective-lockstep boundary only after
/// being converted to a typed fact with an explicit audience.
pub fn retained_event_to_fact(
    ordered: &OrderedOutput<RuntimeEvent>,
) -> Option<OrderedFact> {
    let event = &ordered.value;
    match (event.kind.as_str(), event.action.as_str()) {
        ("game", "end") => Some(OrderedFact {
            key: FactOrderingKey { fact_kind: FactKind::Terminal, ..ordered.key },
            audience: FactAudience::AllPlayers,
            fact: ObservableFact::Terminal {
                result_code: stable_text_id(
                    event.data.get("result").and_then(|value| value.as_str()).unwrap_or("unknown"),
                ) as u32,
                winning_team: event.data.get("winning_team").and_then(|value| value.as_u64()).map(|v| v as u32),
            },
        }),
        ("game", _) => Some(OrderedFact {
            key: FactOrderingKey { fact_kind: FactKind::Hud, ..ordered.key },
            audience: event.data.get("team").and_then(|value| value.as_u64())
                .map(|team| FactAudience::Team(team as u32))
                .unwrap_or(FactAudience::AllPlayers),
            fact: ObservableFact::Hud {
                team: event.data.get("team").and_then(|value| value.as_u64()).unwrap_or(0) as u32,
                metric_id: stable_text_id(&event.action),
                value: event.data.get("value").or_else(|| event.data.get("lives"))
                    .and_then(|value| value.as_i64()).unwrap_or(0),
            },
        }),
        _ => None,
    }
}

fn stable_text_id(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
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
