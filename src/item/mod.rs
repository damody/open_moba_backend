pub use omoba_core::runtime::item::*;

use crate::json_preprocessor::JsonPreprocessor;

pub fn load_registry_from_path(path: &str) -> Result<ItemRegistry, Box<dyn std::error::Error>> {
    let raw = std::fs::read_to_string(path)?;
    let cleaned = JsonPreprocessor::remove_comments(&raw);
    let list: Vec<ItemConfig> = serde_json::from_str(&cleaned)?;
    let registry = ItemRegistry::from_configs(list);
    log::info!("已載入 {} 件裝備", registry.items.len());
    Ok(registry)
}
