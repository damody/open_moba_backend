//! `knowledge_tree.json` 解析器。
//! 讀取 `scripts/lua_data/knowledge_tree.json`，驗證前置節點，
//! 解析失敗時 log 警告並 fallback 為空列表（不 panic）。

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeBonus {
    pub stat_key: String,
    /// Additive 加成（加法 sum_add）。若 JSON 未指定則為 None。
    #[serde(default)]
    pub add: Option<f64>,
    /// Multiplicative 加成（乘法 product_mult）。若 JSON 未指定則為 None。
    #[serde(default)]
    pub multiply: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeNode {
    pub id: String,
    /// 適用類別："tower_dart", "tower_ice", ... "hero", "global"。
    pub category: String,
    pub kp_cost: u32,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub bonuses: Vec<KnowledgeBonus>,
}

/// 載入並驗證 `knowledge_tree.json`。
///
/// 解析失敗或路徑不存在時回傳空 Vec 並 log 警告（不 panic）。
/// 前置節點 id 不存在的節點會被跳過並 log 警告。
pub fn load_knowledge_tree(lua_data_root: &Path) -> Vec<KnowledgeNode> {
    let path = lua_data_root.join("knowledge_tree.json");
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            log::warn!(
                "[general_knowledge] 無法讀取 {:?}：{}；以空知識樹繼續。",
                path,
                e
            );
            return Vec::new();
        }
    };
    let nodes: Vec<KnowledgeNode> = match serde_json::from_str(&raw) {
        Ok(n) => n,
        Err(e) => {
            log::warn!(
                "[general_knowledge] 解析 {:?} 失敗：{}；以空知識樹繼續。",
                path,
                e
            );
            return Vec::new();
        }
    };

    validate_nodes(nodes)
}

/// 驗證 requires 欄位，過濾掉引用不存在節點的項目。
fn validate_nodes(nodes: Vec<KnowledgeNode>) -> Vec<KnowledgeNode> {
    let ids: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let mut result = Vec::with_capacity(nodes.len());
    for node in nodes {
        let bad_req: Vec<&str> = node
            .requires
            .iter()
            .filter(|r| !ids.contains(r.as_str()))
            .map(|r| r.as_str())
            .collect();
        if !bad_req.is_empty() {
            log::warn!(
                "[general_knowledge] 節點 '{}' 的 requires {:?} 引用不存在的節點，跳過此節點。",
                node.id,
                bad_req
            );
        } else {
            result.push(node);
        }
    }
    result
}

/// 將已解鎖節點列表與知識樹合併，計算出「category → Vec<(buff_id, json_payload)>」映射。
/// 供 `KnowledgeBonusResource` 使用。
pub fn build_bonus_map(
    tree: &[KnowledgeNode],
    unlocked: &[String],
) -> HashMap<String, Vec<(String, String)>> {
    let unlocked_set: HashSet<&str> = unlocked.iter().map(|s| s.as_str()).collect();
    let mut map: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for node in tree {
        if !unlocked_set.contains(node.id.as_str()) {
            continue;
        }
        let buff_id = format!("gk_{}", node.id);
        let payload = build_payload_json(&node.bonuses);
        if payload == "{}" {
            continue;
        }
        map.entry(node.category.clone())
            .or_default()
            .push((buff_id, payload));
    }
    map
}

/// `bonuses` 陣列 → `{"stat_key": raw_value, ...}` JSON 字串。
/// Fixed64 raw 值 = f64 * 1024，以整數形式儲存（Phase 1de.2 格式）。
fn build_payload_json(bonuses: &[KnowledgeBonus]) -> String {
    let mut obj = serde_json::Map::new();
    for b in bonuses {
        if let Some(add) = b.add {
            let raw = (add * 1024.0) as i64;
            obj.insert(b.stat_key.clone(), serde_json::Value::Number(raw.into()));
        }
        if let Some(mul) = b.multiply {
            let key = if b.stat_key.ends_with("_multiplier") {
                b.stat_key.clone()
            } else {
                format!("{}_multiplier", b.stat_key)
            };
            let raw = (mul * 1024.0) as i64;
            obj.insert(key, serde_json::Value::Number(raw.into()));
        }
    }
    serde_json::Value::Object(obj).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn with_temp_dir(name: &str, f: impl FnOnce(&Path)) {
        let dir = std::env::temp_dir().join(format!("gk_test_{}", name));
        let _ = std::fs::create_dir_all(&dir);
        f(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn write_json(dir: &Path, content: &str) {
        let mut file = std::fs::File::create(dir.join("knowledge_tree.json")).unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn load_valid_tree() {
        with_temp_dir("load_valid", |dir| {
            let json = r#"[
                {"id":"n1","category":"tower_dart","kp_cost":3,"requires":[],"bonuses":[{"stat_key":"bonus_damage","add":5.0}]},
                {"id":"n2","category":"tower_dart","kp_cost":5,"requires":["n1"],"bonuses":[]}
            ]"#;
            write_json(dir, json);
            let nodes = load_knowledge_tree(dir);
            assert_eq!(nodes.len(), 2);
            assert_eq!(nodes[0].id, "n1");
        });
    }

    #[test]
    fn load_corrupt_json_returns_empty() {
        with_temp_dir("corrupt", |dir| {
            write_json(dir, "NOT_JSON{{{{");
            let nodes = load_knowledge_tree(dir);
            assert!(nodes.is_empty());
        });
    }

    #[test]
    fn load_missing_file_returns_empty() {
        with_temp_dir("missing", |dir| {
            // 不建立 knowledge_tree.json
            let nodes = load_knowledge_tree(dir);
            assert!(nodes.is_empty());
        });
    }

    #[test]
    fn bad_requires_node_skipped() {
        with_temp_dir("bad_req", |dir| {
            let json = r#"[
                {"id":"n1","category":"tower_dart","kp_cost":3,"requires":["NONEXISTENT"],"bonuses":[]},
                {"id":"n2","category":"tower_dart","kp_cost":3,"requires":[],"bonuses":[]}
            ]"#;
            write_json(dir, json);
            let nodes = load_knowledge_tree(dir);
            // n1 skipped (bad requires), n2 passes
            assert_eq!(nodes.len(), 1);
            assert_eq!(nodes[0].id, "n2");
        });
    }

    #[test]
    fn build_bonus_map_unlocked_only() {
        let tree = vec![
            KnowledgeNode {
                id: "n1".to_string(),
                category: "tower_dart".to_string(),
                kp_cost: 3,
                requires: vec![],
                bonuses: vec![KnowledgeBonus {
                    stat_key: "bonus_damage".to_string(),
                    add: Some(5.0),
                    multiply: None,
                }],
            },
            KnowledgeNode {
                id: "n2".to_string(),
                category: "global".to_string(),
                kp_cost: 8,
                requires: vec![],
                bonuses: vec![KnowledgeBonus {
                    stat_key: "range_bonus".to_string(),
                    add: Some(10.0),
                    multiply: None,
                }],
            },
        ];
        let unlocked = vec!["n1".to_string()];
        let map = build_bonus_map(&tree, &unlocked);
        assert!(map.contains_key("tower_dart"));
        assert!(!map.contains_key("global")); // n2 not unlocked
        let dart_buffs = &map["tower_dart"];
        assert_eq!(dart_buffs.len(), 1);
        assert_eq!(dart_buffs[0].0, "gk_n1");
    }
}
