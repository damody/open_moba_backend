//! TOML has no null: named optional slots preserve genuinely unequipped slots.
//! Keep the existing two-element array decoder for older server-owned inputs.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;

type Loadouts = BTreeMap<u32, [Option<String>; 2]>;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NamedSlots {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slot0: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slot1: Option<String>,
}

enum Slots {
    Named(NamedSlots),
    Legacy([Option<String>; 2]),
}

impl<'de> Deserialize<'de> for Slots {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Slots;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a named slot map or exactly two legacy slots")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<Slots, A::Error> {
                NamedSlots::deserialize(serde::de::value::MapAccessDeserializer::new(map)).map(Slots::Named)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Slots, A::Error> {
                let first = sequence.next_element::<Option<String>>()?
                    .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
                let second = sequence.next_element::<Option<String>>()?
                    .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
                if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(3, &self));
                }
                Ok(Slots::Legacy([first, second]))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

pub fn serialize<S: Serializer>(value: &Loadouts, serializer: S) -> Result<S::Ok, S::Error> {
    let named: BTreeMap<_, _> = value.iter().map(|(&player, slots)| {
        (player.to_string(), NamedSlots { slot0: slots[0].clone(), slot1: slots[1].clone() })
    }).collect();
    named.serialize(serializer)
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Loadouts, D::Error> {
    let named = BTreeMap::<String, Slots>::deserialize(deserializer)?;
    named.into_iter().map(|(key, slots)| {
        let player: u32 = key.parse().map_err(serde::de::Error::custom)?;
        if key != player.to_string() {
            return Err(serde::de::Error::custom("summoner player ID must be a canonical unsigned integer"));
        }
        let slots = match slots {
            Slots::Named(value) => [value.slot0, value.slot1],
            Slots::Legacy(value) => value,
        };
        Ok((player, slots))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize, Serialize)]
    struct Configuration {
        #[serde(default, with = "super")]
        loadouts: Loadouts,
    }

    #[test]
    fn summoner_toml_preserves_each_empty_slot_and_legacy_arrays() {
        let input = r#"
loadouts = { "1" = { slot0 = "training_point_blink_v1", slot1 = "training_self_heal_v1" }, "2" = { slot0 = "training_self_heal_v1" }, "3" = { slot1 = "training_point_blink_v1" }, "4" = {} }
"#;
        let value: Configuration = toml::from_str(input).unwrap();
        assert!(value.loadouts[&1].iter().all(Option::is_some));
        assert_eq!(value.loadouts[&2], [Some("training_self_heal_v1".into()), None]);
        assert_eq!(value.loadouts[&3], [None, Some("training_point_blink_v1".into())]);
        assert_eq!(value.loadouts[&4], [None, None]);
        let encoded = toml::to_string(&value).unwrap();
        assert!(!encoded.contains("null"));
        assert_eq!(toml::from_str::<Configuration>(&encoded).unwrap().loadouts, value.loadouts);
        let legacy: Configuration = toml::from_str(r#"loadouts = { "1" = ["training_point_blink_v1", "training_self_heal_v1"] }"#).unwrap();
        assert_eq!(legacy.loadouts[&1], value.loadouts[&1]);
        assert!(toml::from_str::<Configuration>("").unwrap().loadouts.is_empty());
        // The JSON config boundary still supports old arrays with explicit null.
        let legacy_json: Configuration = serde_json::from_str(r#"{"loadouts":{"1":[null,"training_self_heal_v1"]}}"#).unwrap();
        assert_eq!(legacy_json.loadouts[&1], [None, Some("training_self_heal_v1".into())]);
    }

    #[test]
    fn summoner_toml_rejects_ambiguous_or_malformed_slots() {
        for input in [
            r#"loadouts = { "1" = ["training_point_blink_v1"] }"#,
            r#"loadouts = { "1" = ["a", "b", "c"] }"#,
            r#"loadouts = { "1" = { slot2 = "a" } }"#,
            r#"loadouts = { "1" = { slot0 = 1 } }"#,
            r#"loadouts = { "01" = {} }"#,
            r#"loadouts = { "-1" = {} }"#,
            r#"loadouts = { "4294967296" = {} }"#,
            r#"loadouts = { "1" = { slot0 = "a", slot0 = "b" } }"#,
        ] {
            assert!(toml::from_str::<Configuration>(input).is_err(), "accepted {input}");
        }
    }
}
