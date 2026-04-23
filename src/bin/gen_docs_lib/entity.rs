//! Load hero / creep base stats from omb/Story/<STORY>/entity.json.

use crate::lib::model::{HeroInfo, CreepInfo};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

pub struct EntityData {
    pub heroes: BTreeMap<String, HeroInfo>,
    pub creeps: BTreeMap<String, CreepInfo>,
}

pub fn load(story_dir: &Path) -> Result<EntityData> {
    let path = story_dir.join("entity.json");
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    let cleaned = strip_line_comments(&raw);
    let v: serde_json::Value = serde_json::from_str(&cleaned)
        .with_context(|| format!("parsing {}", path.display()))?;
    parse(v)
}

fn strip_line_comments(src: &str) -> String {
    src.lines()
        .map(strip_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 掃一行 JSON，砍掉第一個「字串字面值外」的 `//` 之後所有內容。
/// 字串字面值：雙引號包起來，`\"` 視為 escape。
fn strip_line(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut in_str = false;
    let mut escape = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else {
            if c == b'"' {
                in_str = true;
            } else if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                // 字串外的 // → 截掉
                return line[..i].trim_end().to_string();
            }
        }
        i += 1;
    }
    line.to_string()
}

fn parse(v: serde_json::Value) -> Result<EntityData> {
    let mut heroes = BTreeMap::new();
    let mut creeps = BTreeMap::new();
    if let Some(arr) = v.get("heroes").and_then(|x| x.as_array()) {
        for h in arr {
            let id = h.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if id.is_empty() { continue; }
            heroes.insert(id.clone(), parse_hero(h));
        }
    }
    if let Some(arr) = v.get("enemies").and_then(|x| x.as_array()) {
        for c in arr {
            let id = c.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if id.is_empty() { continue; }
            creeps.insert(id.clone(), parse_creep(c));
        }
    }
    Ok(EntityData { heroes, creeps })
}

fn f(v: &serde_json::Value, key: &str) -> f32 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32
}
fn s(v: &serde_json::Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}
fn i(v: &serde_json::Value, key: &str) -> i32 {
    v.get(key).and_then(|x| x.as_i64()).unwrap_or(0) as i32
}
fn arr_str(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key).and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default()
}

fn parse_hero(v: &serde_json::Value) -> HeroInfo {
    HeroInfo {
        name: s(v, "name"),
        title: s(v, "title"),
        background: s(v, "background"),
        strength: f(v, "strength"),
        agility: f(v, "agility"),
        intelligence: f(v, "intelligence"),
        primary_attribute: s(v, "primary_attribute"),
        attack_range: f(v, "attack_range"),
        base_damage: f(v, "base_damage"),
        base_armor: f(v, "base_armor"),
        base_hp: f(v, "base_hp"),
        base_mana: f(v, "base_mana"),
        move_speed: f(v, "move_speed"),
        turn_speed: f(v, "turn_speed"),
        abilities: arr_str(v, "abilities"),
        level_growth: v.get("level_growth").cloned().unwrap_or(serde_json::Value::Null),
    }
}

fn parse_creep(v: &serde_json::Value) -> CreepInfo {
    CreepInfo {
        name: s(v, "name"),
        enemy_type: s(v, "enemy_type"),
        hp: f(v, "hp"),
        armor: f(v, "armor"),
        magic_resistance: f(v, "magic_resistance"),
        damage: f(v, "damage"),
        attack_range: f(v, "attack_range"),
        move_speed: f(v, "move_speed"),
        ai_type: s(v, "ai_type"),
        abilities: arr_str(v, "abilities"),
        exp_reward: i(v, "exp_reward"),
        gold_reward: i(v, "gold_reward"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_removes_line_leading_comment() {
        let src = "{\n  // hello\n  \"a\": 1\n}";
        let out = strip_line_comments(src);
        assert!(!out.contains("hello"));
        assert!(out.contains("\"a\": 1"));
    }

    #[test]
    fn strip_preserves_double_slash_inside_string() {
        let src = "{\n  \"url\": \"http://x\"\n}";
        let out = strip_line_comments(src);
        assert!(out.contains("http://x"));
    }

    #[test]
    fn strip_cuts_trailing_comment_but_keeps_json() {
        let src = "{\n  \"hp\": 100, // 100hp\n  \"dmg\": 5\n}";
        let out = strip_line_comments(src);
        assert!(!out.contains("100hp"));
        // 原本那行前半 "hp": 100, 必須保留
        assert!(out.contains("\"hp\": 100,"));
        // 下一行不受影響
        assert!(out.contains("\"dmg\": 5"));
    }

    #[test]
    fn parses_hero_and_creep() {
        let raw = r#"{
            "heroes": [{"id":"h1","name":"Hero","base_hp":500,"abilities":["a"]}],
            "enemies": [{"id":"c1","name":"Creep","hp":300,"damage":20}]
        }"#;
        let d = parse(serde_json::from_str(raw).unwrap()).unwrap();
        let h = d.heroes.get("h1").unwrap();
        assert_eq!(h.name, "Hero");
        assert_eq!(h.base_hp, 500.0);
        assert_eq!(h.abilities, vec!["a".to_string()]);
        let c = d.creeps.get("c1").unwrap();
        assert_eq!(c.hp, 300.0);
        assert_eq!(c.damage, 20.0);
    }
}
