use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::comp::circular_vision::VisionResult;

/// 視野計算緩存項
#[derive(Debug, Clone)]
pub struct VisionCache {
    pub result: VisionResult,
    pub last_update: f64,
    pub dependencies: Vec<String>, // 依賴的障礙物ID
}

/// 緩存管理器
pub struct CacheManager {
    /// 視野結果緩存
    vision_cache: HashMap<String, VisionCache>,
    /// 緩存大小限制
    max_cache_size: usize,
}

impl CacheManager {
    pub fn new(max_cache_size: usize) -> Self {
        Self {
            vision_cache: HashMap::new(),
            max_cache_size,
        }
    }

    /// 獲取緩存結果
    pub fn get_cached_vision(&self, cache_key: &str) -> Option<&VisionCache> {
        if let Some(cached) = self.vision_cache.get(cache_key) {
            let current_time = self.current_time();
            // 緩存有效期：1秒
            if current_time - cached.last_update < 1.0 {
                return Some(cached);
            }
        }
        None
    }

    /// 緩存視野結果
    pub fn cache_vision_result(&mut self, cache_key: String, result: VisionResult, dependencies: Vec<String>) {
        let cache_entry = VisionCache {
            result,
            last_update: self.current_time(),
            dependencies,
        };

        self.vision_cache.insert(cache_key, cache_entry);
        self.limit_cache_size();
    }

    /// 使特定障礙物相關的緩存失效
    pub fn invalidate_cache_for_obstacle(&mut self, obstacle_id: &str) {
        self.vision_cache.retain(|_, cache| {
            !cache.dependencies.contains(&obstacle_id.to_string())
        });
    }

    /// 清理所有緩存
    pub fn invalidate_all_cache(&mut self) {
        self.vision_cache.clear();
    }

    /// 限制緩存大小
    fn limit_cache_size(&mut self) {
        while self.vision_cache.len() > self.max_cache_size {
            // 移除最舊的緩存項
            let oldest_key = self.vision_cache
                .iter()
                .min_by(|a, b| a.1.last_update.partial_cmp(&b.1.last_update).unwrap())
                .map(|(k, _)| k.clone());
                
            if let Some(key) = oldest_key {
                self.vision_cache.remove(&key);
            } else {
                break;
            }
        }
    }

    /// 獲取當前時間戳
    fn current_time(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    /// 獲取緩存統計
    pub fn get_cache_stats(&self) -> CacheStats {
        CacheStats {
            cache_size: self.vision_cache.len(),
            max_cache_size: self.max_cache_size,
        }
    }
}

/// 緩存統計信息
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub cache_size: usize,
    pub max_cache_size: usize,
}