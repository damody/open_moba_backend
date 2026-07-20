//! 玩家跨局持久化 Profile — 儲存 KP 累積與已解鎖知識節點。
//! 持久化至 `omb/player_profile.json`。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::loader::KnowledgeNode;

const PROFILE_FILENAME: &str = "player_profile.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlayerProfile {
    /// 累計獲得的知識點。
    pub total_kp: u32,
    /// 已消耗的知識點（已解鎖節點 kp_cost 之和）。
    pub spent_kp: u32,
    /// 已解鎖節點的 id 列表。
    pub unlocked_nodes: Vec<String>,
}

impl PlayerProfile {
    /// 可用知識點 = total_kp - spent_kp。
    pub fn available_kp(&self) -> u32 {
        self.total_kp.saturating_sub(self.spent_kp)
    }

    /// 是否已解鎖指定節點。
    pub fn is_unlocked(&self, node_id: &str) -> bool {
        self.unlocked_nodes.iter().any(|id| id == node_id)
    }
}

fn profile_path(omb_dir: &Path) -> PathBuf {
    omb_dir.join(PROFILE_FILENAME)
}

/// 從 `omb_dir/player_profile.json` 讀取 profile。
/// 檔案不存在 → 回傳空 profile。
/// 解析失敗 → log 警告，回傳空 profile。
pub fn load_profile(omb_dir: &Path) -> PlayerProfile {
    let path = profile_path(omb_dir);
    match std::fs::read_to_string(&path) {
        Err(_) => {
            // 不存在是正常情況（首次啟動）
            PlayerProfile::default()
        }
        Ok(raw) => match serde_json::from_str::<PlayerProfile>(&raw) {
            Ok(p) => p,
            Err(e) => {
                log::warn!(
                    "[general_knowledge] player_profile.json 解析失敗 ({})，重置為空 profile。",
                    e
                );
                PlayerProfile::default()
            }
        },
    }
}

/// 將 profile 寫回 `omb_dir/player_profile.json`。
/// 寫入失敗時 log 警告（不 panic）。
pub fn save_profile(omb_dir: &Path, profile: &PlayerProfile) {
    let path = profile_path(omb_dir);
    match serde_json::to_string_pretty(profile) {
        Err(e) => log::warn!("[general_knowledge] 序列化 profile 失敗：{}", e),
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                log::warn!("[general_knowledge] 寫入 {:?} 失敗：{}", path, e);
            }
        }
    }
}

/// 解鎖指定節點。
///
/// 失敗時回傳 `Err` 說明原因（KP 不足 / 前置節點未解鎖 / 節點不存在）。
/// 成功時更新 profile 並寫回磁碟，回傳 `Ok(())`。
pub fn unlock_node(
    omb_dir: &Path,
    profile: &mut PlayerProfile,
    tree: &[KnowledgeNode],
    node_id: &str,
) -> Result<(), String> {
    let node = tree
        .iter()
        .find(|n| n.id == node_id)
        .ok_or_else(|| format!("節點 '{}' 不存在於知識樹", node_id))?;

    if profile.is_unlocked(node_id) {
        return Err(format!("節點 '{}' 已解鎖", node_id));
    }

    // 驗證前置節點
    for req in &node.requires {
        if !profile.is_unlocked(req) {
            return Err(format!(
                "節點 '{}' 的前置節點 '{}' 尚未解鎖",
                node_id, req
            ));
        }
    }

    // 驗證 KP
    if profile.available_kp() < node.kp_cost {
        return Err(format!(
            "KP 不足：需要 {} 但只有 {}",
            node.kp_cost,
            profile.available_kp()
        ));
    }

    profile.spent_kp += node.kp_cost;
    profile.unlocked_nodes.push(node_id.to_string());
    save_profile(omb_dir, profile);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::loader::KnowledgeBonus;

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gk_profile_test_{}", name));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    fn make_tree() -> Vec<KnowledgeNode> {
        vec![
            KnowledgeNode {
                id: "n1".to_string(),
                category: "global".to_string(),
                kp_cost: 3,
                requires: vec![],
                bonuses: vec![],
            },
            KnowledgeNode {
                id: "n2".to_string(),
                category: "global".to_string(),
                kp_cost: 5,
                requires: vec!["n1".to_string()],
                bonuses: vec![],
            },
        ]
    }

    #[test]
    fn load_missing_returns_default() {
        let dir = tmp_dir("missing");
        let p = load_profile(&dir);
        assert_eq!(p.total_kp, 0);
        assert_eq!(p.unlocked_nodes.len(), 0);
        cleanup(&dir);
    }

    #[test]
    fn save_and_reload() {
        let dir = tmp_dir("save_reload");
        let mut p = PlayerProfile::default();
        p.total_kp = 100;
        p.unlocked_nodes.push("n1".to_string());
        save_profile(&dir, &p);
        let p2 = load_profile(&dir);
        assert_eq!(p2.total_kp, 100);
        assert!(p2.is_unlocked("n1"));
        cleanup(&dir);
    }

    #[test]
    fn unlock_ok() {
        let dir = tmp_dir("unlock_ok");
        let tree = make_tree();
        let mut p = PlayerProfile { total_kp: 10, spent_kp: 0, unlocked_nodes: vec![] };
        unlock_node(&dir, &mut p, &tree, "n1").unwrap();
        assert!(p.is_unlocked("n1"));
        assert_eq!(p.spent_kp, 3);
        cleanup(&dir);
    }

    #[test]
    fn unlock_insufficient_kp() {
        let dir = tmp_dir("unlock_kp");
        let tree = make_tree();
        let mut p = PlayerProfile { total_kp: 2, spent_kp: 0, unlocked_nodes: vec![] };
        let err = unlock_node(&dir, &mut p, &tree, "n1").unwrap_err();
        assert!(err.contains("KP 不足"));
        cleanup(&dir);
    }

    #[test]
    fn unlock_prereq_not_met() {
        let dir = tmp_dir("unlock_prereq");
        let tree = make_tree();
        let mut p = PlayerProfile { total_kp: 100, spent_kp: 0, unlocked_nodes: vec![] };
        let err = unlock_node(&dir, &mut p, &tree, "n2").unwrap_err();
        assert!(err.contains("前置節點"));
        cleanup(&dir);
    }
}
