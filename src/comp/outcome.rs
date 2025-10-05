// 引入必要的模組和套件
use crate::{comp, Creep, CProperty, TProperty};
use super::Projectile;
use hashbrown::HashSet;
use serde::{Deserialize, Serialize};
use vek::*;  // 向量數學庫
use specs::Entity;  // ECS 實體系統
use std::collections::VecDeque;
use std::sync::Mutex;
use std::ops::DerefMut;
use std::cmp::Ordering;
use voracious_radix_sort::{Radixable, RadixSort};  // 基數排序演算法
use crate::Tower;
use crate::TAttack;

/// 遊戲結果事件枚舉
/// 用於處理遊戲中各種事件的結果，例如傷害、死亡、治療等
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Outcome {
    /// 傷害事件
    Damage {
        pos: Vec2<f32>,      // 傷害發生位置
        phys: f32,           // 物理傷害數值
        magi: f32,           // 魔法傷害數值
        real: f32,           // 真實傷害數值（無視防禦）
        source: Entity,      // 傷害來源實體
        target: Entity,      // 傷害目標實體
    },
    /// 投射物軌跡事件
    ProjectileLine2 {
        pos: Vec2<f32>,                // 投射物位置
        source: Option<Entity>,        // 投射物來源實體（可選）
        target: Option<Entity>,        // 投射物目標實體（可選）
    },
    /// 死亡事件
    Death {
        pos: Vec2<f32>,      // 死亡位置
        ent: Entity,         // 死亡的實體
    },
    /// 小兵生成事件
    Creep {
        cd: CreepData,       // 小兵資料
    },
    /// 小兵停止移動事件
    CreepStop {
        source: Entity,      // 發起停止的實體
        target: Entity,      // 目標實體
    },
    /// 小兵移動事件
    CreepWalk {
        target: Entity,      // 移動的目標實體
    },
    /// 塔防建築事件
    Tower {
        pos: Vec2<f32>,      // 塔的位置
        td: TowerData,       // 塔的資料
    },
    /// 治療事件
    Heal {
        pos: Vec2<f32>,      // 治療發生位置
        target: Entity,      // 治療目標實體
        amount: f32,         // 治療量
    },
    /// 更新攻擊狀態事件
    UpdateAttack {
        target: Entity,                  // 目標實體
        asd_count: Option<f32>,          // 攻擊速度計數器（可選）
        cooldown_reset: bool,            // 是否重置冷卻時間
    },
    /// 獲得經驗值事件
    GainExperience {
        target: Entity,      // 獲得經驗的實體
        amount: i32,         // 經驗值數量
    },
    /// 生成單位事件
    SpawnUnit {
        pos: Vec2<f32>,                        // 生成位置
        unit: crate::comp::Unit,               // 單位類型
        faction: crate::comp::Faction,         // 陣營
        duration: Option<f32>,                 // 持續時間（可選，用於臨時單位）
    }
}

/// 小兵資料結構
/// 儲存小兵的相關資訊
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreepData {
    pub pos: Vec2<f32>,      // 小兵位置
    pub creep: Creep,         // 小兵基本資料
    pub cdata: CProperty,     // 小兵屬性資料
}

/// 塔防建築資料結構
/// 儲存塔的相關資訊
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TowerData {
    pub tpty: TProperty,      // 塔的屬性資料
    pub tatk: TAttack,        // 塔的攻擊資料
}

/// X軸位置索引結構
/// 用於根據X座標進行排序和搜尋
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct PosXIndex {
    pub e: Entity,           // 實體參考
    pub p: Vec2<f32>,        // 位置向量
}

/// Y軸位置索引結構
/// 用於根據Y座標進行排序和搜尋
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct PosYIndex {
    pub e: Entity,           // 實體參考
    pub p: Vec2<f32>,        // 位置向量
}

// 實作 PosXIndex 的部分排序功能
// 根據 X 座標進行比較
impl PartialOrd for PosXIndex {
    fn partial_cmp(&self, other: &PosXIndex) -> Option<Ordering> {
        self.p.x.partial_cmp(&other.p.x)
    }
}

// 實作 PosXIndex 的相等比較
// 只比較 X 座標是否相等
impl PartialEq for PosXIndex {
    fn eq(&self, other: &Self) -> bool {
        self.p.x == other.p.x
    }
}

// 實作 PosXIndex 的基數排序介面
// 使用 X 座標作為排序鍵值
impl Radixable<f32> for PosXIndex {
    type Key = f32;
    #[inline]
    fn key(&self) -> Self::Key {
        self.p.x
    }
}

// 實作 PosYIndex 的部分排序功能
// 根據 Y 座標進行比較
impl PartialOrd for PosYIndex {
    fn partial_cmp(&self, other: &PosYIndex) -> Option<Ordering> {
        self.p.y.partial_cmp(&other.p.y)
    }
}

// 實作 PosYIndex 的相等比較
// 只比較 Y 座標是否相等
impl PartialEq for PosYIndex {
    fn eq(&self, other: &Self) -> bool {
        self.p.y == other.p.y
    }
}

// 實作 PosYIndex 的基數排序介面
// 使用 Y 座標作為排序鍵值
impl Radixable<f32> for PosYIndex {
    type Key = f32;
    #[inline]
    fn key(&self) -> Self::Key {
        self.p.y
    }
}
/// 距離索引結構
/// 用於根據距離進行排序，主要用於尋找最近的實體
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct DisIndex {
    pub e: Entity,           // 實體參考
    pub dis: f32,            // 距離值（通常是平方距離以避免開根號運算）
}

// 實作 Eq trait，允許完全相等比較
impl Eq for DisIndex {}

// 實作完整排序功能
// 根據距離進行排序
impl Ord for DisIndex {
    fn cmp(&self, other: &Self) -> Ordering{
        self.dis.partial_cmp(&other.dis).unwrap()
    }
}

// 實作部分排序功能
impl PartialOrd for DisIndex {
    fn partial_cmp(&self, other: &DisIndex) -> Option<Ordering> {
        self.dis.partial_cmp(&other.dis)
    }
}

// 實作相等比較
// 只比較距離是否相等
impl PartialEq for DisIndex {
    fn eq(&self, other: &Self) -> bool {
        self.dis == other.dis
    }
}

// 實作基數排序介面
// 使用距離作為排序鍵值
impl Radixable<f32> for DisIndex {
    type Key = f32;
    #[inline]
    fn key(&self) -> Self::Key {
        self.dis
    }
}

/// 第二種距離索引結構
/// 儲存實體及其位置，根據實體ID進行排序
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct DisIndex2 {
    pub e: Entity,           // 實體參考
    pub p: Vec2<f32>,        // 實體位置
}

// 實作 Eq trait
impl Eq for DisIndex2 {}

// 實作完整排序功能
// 根據實體ID進行排序，而非距離
impl Ord for DisIndex2 {
    fn cmp(&self, other: &Self) -> Ordering{
        self.e.cmp(&other.e)
    }
}

// 實作部分排序功能
impl PartialOrd for DisIndex2 {
    fn partial_cmp(&self, other: &DisIndex2) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// 實作相等比較
// 比較實體ID是否相等
impl PartialEq for DisIndex2 {
    fn eq(&self, other: &Self) -> bool {
        self.e == other.e
    }
}

// 實作基數排序介面
// 使用實體ID作為排序鍵值
impl Radixable<u32> for DisIndex2 {
    type Key = u32;
    #[inline]
    fn key(&self) -> Self::Key {
        self.e.id()
    }
}

fn intersect_sorted_iters<T: Ord + Copy>(
    mut a: impl Iterator<Item = T>,
    mut b: impl Iterator<Item = T>,
) -> Vec<T> {
    let mut result = Vec::new();
    let mut peek_a = a.next();
    let mut peek_b = b.next();

    while let (Some(av), Some(bv)) = (peek_a, peek_b) {
        if av == bv {
            result.push(av);
            peek_a = a.next();
            peek_b = b.next();
        } else if av < bv {
            peek_a = a.next();
        } else {
            peek_b = b.next();
        }
    }
    result
}


/// 搜尋器結構
/// 用於高效搜尋遊戲中的塔和小兵實體
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Searcher {
    pub tower: PosData,      // 塔的位置資料
    pub creep: PosData,      // 小兵的位置資料
}

/// 位置資料結構
/// 儲存按X和Y座標排序的實體索引，用於快速空間查詢
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PosData {
    pub xpos: Vec<PosXIndex>,    // 按X座標排序的索引陣列
    pub ypos: Vec<PosYIndex>,    // 按Y座標排序的索引陣列
    pub needsort: bool,          // 標記是否需要重新排序
}
impl PosData {
    /// 建立新的位置資料結構
    pub fn new() -> PosData {
        PosData {
            xpos: vec![],        // 初始化空的X座標索引陣列
            ypos: vec![],        // 初始化空的Y座標索引陣列
            needsort: false,     // 初始時不需要排序
        }
    }
    
    /// 搜尋最近鄰居（使用兩個半徑範圍）
    /// 參數：
    /// - pos: 搜尋中心位置
    /// - radius1: 內圈半徑（用於篩選近距離實體）
    /// - radius2: 外圈半徑（用於初步篩選範圍）
    /// - n: 返回的最大實體數量
    /// 返回：內圈和外圈的實體列表
    pub fn SearchNN_XY2(&self, pos: Vec2<f32>, radius1: f32, radius2: f32, n: usize) -> (Vec<DisIndex>, Vec<DisIndex>) {
        let r2 = radius1*radius1;  // 計算內圈半徑的平方（避免開根號）
        let mut res = vec![];       // 內圈結果
        let mut res2 = vec![];      // 外圈結果
        let mut xdata = vec![];     // X軸範圍內的實體
        let mut ydata = vec![];     // Y軸範圍內的實體
        
        // 計算X軸搜尋範圍
        let lx = pos.x - radius2;   // 左邊界
        let rx = pos.x + radius2;   // 右邊界
        
        // 使用二分搜尋找到X軸範圍的起始索引
        let lxp = self.xpos.binary_search_by(|data| data.p.x.partial_cmp(&lx).unwrap());
        let lxi = match lxp {
            Ok(x) => {x}
            Err(x) => {x}
        };
        
        // 使用二分搜尋找到X軸範圍的結束索引
        let rxp = self.xpos.binary_search_by(|data| data.p.x.partial_cmp(&rx).unwrap());
        let rxi = match rxp {
            Ok(x) => {x}
            Err(x) => {x}
        };
        
        // 收集X軸範圍內的實體
        for i in lxi..rxi {
            if let Some(p) = self.xpos.get(i) {  // 修正：從 xpos 取資料而不是 ypos
                xdata.push(DisIndex2 { e: p.e, p: p.p });
            }
        }
        
        // 計算Y軸搜尋範圍
        let ly = pos.y - radius2;   // 下邊界
        let ry = pos.y + radius2;   // 上邊界
        
        // 使用二分搜尋找到Y軸範圍的起始索引
        let lyp = self.ypos.binary_search_by(|data| data.p.y.partial_cmp(&ly).unwrap());
        let lyi = match lyp {
            Ok(y) => {y}
            Err(y) => {y}
        };
        
        // 使用二分搜尋找到Y軸範圍的結束索引
        let ryp = self.ypos.binary_search_by(|data| data.p.y.partial_cmp(&ry).unwrap());
        let ryi = match ryp {
            Ok(y) => {y}
            Err(y) => {y}
        };
        
        // 收集Y軸範圍內的實體
        for i in lyi..ryi {
            if let Some(p) = self.ypos.get(i) {
                ydata.push(DisIndex2 { e: p.e, p: p.p });
            }
        }
        
        // 對兩個資料集進行多執行緒基數排序
        xdata.voracious_mt_sort(4);
        ydata.voracious_mt_sort(4);
        
        // 找出X和Y範圍的交集（同時在兩個範圍內的實體）
        let mut ary = [xdata.iter(), ydata.iter()];
        let intersection_iter = intersect_sorted_iters(xdata.iter(), ydata.iter());
        
        // 計算實際距離並分類到內圈或外圈
        for p in intersection_iter {
            let dis = p.p.distance_squared(pos);
            if dis < r2 {
                res.push(DisIndex { e: p.e, dis: dis });  // 內圈實體
            } else {
                res2.push(DisIndex { e: p.e, dis: dis }); // 外圈實體
            }
        }
        
        // 對內圈結果按距離排序並截取前n個
        res.voracious_mt_sort(4);
        res.truncate(n);
        (res, res2)
    }
    /// 搜尋最近鄰居（使用XY軸交集法）
    /// 參數：
    /// - pos: 搜尋中心位置
    /// - radius: 搜尋半徑
    /// - n: 返回的最大實體數量
    /// 返回：範圍內最近的n個實體
    pub fn SearchNN_XY(&self, pos: Vec2<f32>, radius: f32, n: usize) -> Vec<DisIndex> {
        let r2 = radius*radius;     // 計算半徑平方（避免開根號）
        let mut res = vec![];       // 結果列表
        let mut xdata = vec![];     // X軸範圍內的實體
        let mut ydata = vec![];     // Y軸範圍內的實體
        
        // 在X軸排序陣列中找到最接近的位置
        let xp = self.xpos.binary_search_by(|data| data.p.x.partial_cmp(&pos.x).unwrap());
        let xidx = match xp {
            Ok(x) => {x}
            Err(x) => {x}
        };
        
        // 向左搜尋X軸範圍內的實體
        let mut loffset = 0;
        let mut roffset = 1;
        loop {
            if let Some(p) = self.xpos.get((xidx as i32 - loffset) as usize) {
                if (p.p.x - pos.x).abs() < radius {
                    xdata.push(DisIndex2 { e: p.e, p: p.p });
                } else {
                    break;  // 超出範圍，停止搜尋
                }
            } else {
                break;  // 到達陣列邊界
            }
            loffset += 1;
        }
        
        // 向右搜尋X軸範圍內的實體
        loop {
            if let Some(p) = self.xpos.get((xidx as i32 + roffset) as usize) {
                if (p.p.x - pos.x).abs() < radius {  // 修正：使用正確的距離計算
                    xdata.push(DisIndex2 { e: p.e, p: p.p });
                } else {
                    break;  // 超出範圍，停止搜尋
                }
            } else {
                break;  // 到達陣列邊界
            }
            roffset += 1;
        }
        
        // 在Y軸排序陣列中找到最接近的位置
        let yp = self.ypos.binary_search_by(|data| data.p.y.partial_cmp(&pos.y).unwrap());
        let yidx = match yp {
            Ok(y) => {y}
            Err(y) => {y}
        };
        
        // 向下搜尋Y軸範圍內的實體
        let mut loffset = 0;
        let mut roffset = 1;
        loop {
            if let Some(p) = self.ypos.get((yidx as i32 - loffset) as usize) {
                if (p.p.y - pos.y).abs() < radius {
                    ydata.push(DisIndex2 { e: p.e, p: p.p });
                } else {
                    break;  // 超出範圍，停止搜尋
                }
            } else {
                break;  // 到達陣列邊界
            }
            loffset += 1;
        }
        
        // 向上搜尋Y軸範圍內的實體
        loop {
            if let Some(p) = self.ypos.get((yidx as i32 + roffset) as usize) {
                if (p.p.y - pos.y).abs() < radius {  // 修正：使用正確的距離計算
                    ydata.push(DisIndex2 { e: p.e, p: p.p });
                } else {
                    break;  // 超出範圍，停止搜尋
                }
            } else {
                break;  // 到達陣列邊界
            }
            roffset += 1;
        }
        
        // 對兩個資料集進行多執行緒基數排序
        xdata.voracious_mt_sort(4);
        ydata.voracious_mt_sort(4);
        
        // 找出X和Y範圍的交集（同時在兩個範圍內的實體）
        let mut ary = [xdata.iter(), ydata.iter()];
        let intersection_iter = intersect_sorted_iters(xdata.iter(), ydata.iter());
        
        // 檢查實際距離是否在半徑內
        for p in intersection_iter {
            let dis = p.p.distance_squared(pos);
            if dis < r2 {
                res.push(DisIndex { e: p.e, dis: dis });
            }
        }
        
        // 按距離排序並截取前n個
        res.voracious_mt_sort(4);
        res.truncate(n);
        res
    }
    /// 搜尋最近鄰居（僅使用X軸索引）
    /// 這是一個簡化版本，只使用X軸索引進行搜尋
    /// 參數：
    /// - pos: 搜尋中心位置
    /// - radius: 搜尋半徑
    /// - n: 返回的最大實體數量
    /// 返回：範圍內最近的n個實體
    pub fn SearchNN_X(&self, pos: Vec2<f32>, radius: f32, n: usize) -> Vec<DisIndex> {
        let r2 = radius*radius;     // 計算半徑平方（避免開根號）
        let mut res = vec![];       // 結果列表
        
        // 在X軸排序陣列中找到最接近的位置
        let xp = self.xpos.binary_search_by(|data| data.p.x.partial_cmp(&pos.x).unwrap());
        let xidx = match xp {
            Ok(x) => {
                x
            }
            Err(x) => {
                x
            }
        };
        
        // 向左搜尋
        let mut loffset = 0;
        let mut roffset = 1;
        loop {
            if let Some(p) = self.xpos.get((xidx as i32 - loffset) as usize) {
                if (p.p.x - pos.x).abs() < radius {  // X軸距離在範圍內
                    let dis = p.p.distance_squared(pos);  // 計算實際距離平方
                    if dis < r2 {  // 確認在圓形範圍內
                        res.push(DisIndex { e: p.e, dis: dis });
                    }
                } else {
                    break;  // X軸距離超出範圍，停止向左搜尋
                }
            } else {
                break;  // 到達陣列左邊界
            }
            loffset += 1;
        }
        
        // 向右搜尋
        loop {
            if let Some(p) = self.xpos.get((xidx as i32 + roffset) as usize) {
                if (p.p.x - pos.x).abs() < radius {  // X軸距離在範圍內
                    let dis = p.p.distance_squared(pos);  // 計算實際距離平方
                    if dis < r2 {  // 確認在圓形範圍內
                        res.push(DisIndex { e: p.e, dis: dis });
                    }
                } else {
                    break;  // X軸距離超出範圍，停止向右搜尋
                }
            } else {
                break;  // 到達陣列右邊界
            }
            roffset += 1;
        }
        
        // 按距離排序並截取前n個
        res.sort_unstable();
        res.truncate(n);
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use specs::{World, WorldExt, Builder};
    
    /// 測試 PosData 結構的功能
    mod posdata_tests {
        use super::*;
        
        /// 建立測試用的實體和位置資料
        fn create_test_data() -> (Vec<Entity>, Vec<Vec2<f32>>) {
            let mut world = World::new();
            let mut entities = vec![];
            let mut positions = vec![];
            
            // 建立測試實體和位置
            // 在 10x10 的網格上建立實體
            for x in 0..5 {
                for y in 0..5 {
                    let entity = world.create_entity().build();
                    entities.push(entity);
                    positions.push(Vec2::new(x as f32 * 2.0, y as f32 * 2.0));
                }
            }
            
            (entities, positions)
        }
        
        /// 測試 PosData::new() 函數
        #[test]
        fn test_posdata_new() {
            let posdata = PosData::new();
            
            // 驗證初始狀態
            assert_eq!(posdata.xpos.len(), 0, "X軸索引應該為空");
            assert_eq!(posdata.ypos.len(), 0, "Y軸索引應該為空");
            assert_eq!(posdata.needsort, false, "初始不需要排序");
        }
        
        /// 測試向 PosData 添加資料並排序
        #[test]
        fn test_posdata_add_and_sort() {
            let mut posdata = PosData::new();
            let (entities, positions) = create_test_data();
            
            // 添加所有實體到索引
            for (entity, pos) in entities.iter().zip(positions.iter()) {
                posdata.xpos.push(PosXIndex { e: *entity, p: *pos });
                posdata.ypos.push(PosYIndex { e: *entity, p: *pos });
            }
            
            // 排序索引
            posdata.xpos.sort_by(|a, b| a.p.x.partial_cmp(&b.p.x).unwrap());
            posdata.ypos.sort_by(|a, b| a.p.y.partial_cmp(&b.p.y).unwrap());
            
            // 驗證排序結果
            for i in 1..posdata.xpos.len() {
                assert!(
                    posdata.xpos[i-1].p.x <= posdata.xpos[i].p.x,
                    "X軸索引應該按升序排列"
                );
            }
            
            for i in 1..posdata.ypos.len() {
                assert!(
                    posdata.ypos[i-1].p.y <= posdata.ypos[i].p.y,
                    "Y軸索引應該按升序排列"
                );
            }
        }
        
        /// 測試 SearchNN_X 函數 - 僅使用X軸索引搜尋
        #[test]
        fn test_search_nn_x() {
            let mut posdata = PosData::new();
            let (entities, positions) = create_test_data();
            
            // 建立索引
            for (entity, pos) in entities.iter().zip(positions.iter()) {
                posdata.xpos.push(PosXIndex { e: *entity, p: *pos });
                posdata.ypos.push(PosYIndex { e: *entity, p: *pos });
            }
            
            // 排序索引
            posdata.xpos.sort_by(|a, b| a.p.x.partial_cmp(&b.p.x).unwrap());
            posdata.ypos.sort_by(|a, b| a.p.y.partial_cmp(&b.p.y).unwrap());
            
            // 測試案例1：搜尋原點附近的實體
            let search_pos = Vec2::new(0.0, 0.0);
            let radius = 3.0;
            let max_results = 5;
            let results = posdata.SearchNN_X(search_pos, radius, max_results);
            
            // 驗證結果
            assert!(results.len() <= max_results, "結果數量不應超過最大值");
            
            // 驗證所有結果都在半徑內
            for result in &results {
                let actual_distance = result.dis.sqrt();
                assert!(
                    actual_distance <= radius,
                    "找到的實體應該在搜尋半徑內"
                );
            }
            
            // 驗證結果按距離排序
            for i in 1..results.len() {
                assert!(
                    results[i-1].dis <= results[i].dis,
                    "結果應該按距離升序排列"
                );
            }
            
            // 測試案例2：搜尋中心點
            let search_pos = Vec2::new(4.0, 4.0);
            let radius = 5.0;
            let max_results = 10;
            let results = posdata.SearchNN_X(search_pos, radius, max_results);
            
            assert!(results.len() > 0, "應該找到至少一個實體");
            assert!(results.len() <= max_results, "結果數量不應超過最大值");
        }
        
        /// 測試 SearchNN_XY 函數 - 使用XY軸交集搜尋
        #[test]
        fn test_search_nn_xy() {
            let mut posdata = PosData::new();
            let (entities, positions) = create_test_data();
            
            // 建立索引
            for (entity, pos) in entities.iter().zip(positions.iter()) {
                posdata.xpos.push(PosXIndex { e: *entity, p: *pos });
                posdata.ypos.push(PosYIndex { e: *entity, p: *pos });
            }
            
            // 排序索引
            posdata.xpos.sort_by(|a, b| a.p.x.partial_cmp(&b.p.x).unwrap());
            posdata.ypos.sort_by(|a, b| a.p.y.partial_cmp(&b.p.y).unwrap());
            
            // 測試案例1：小範圍搜尋
            let search_pos = Vec2::new(2.0, 2.0);
            let radius = 2.5;
            let max_results = 5;
            let results = posdata.SearchNN_XY(search_pos, radius, max_results);
            
            // 驗證結果
            assert!(results.len() > 0, "應該找到至少一個實體");
            assert!(results.len() <= max_results, "結果數量不應超過最大值");
            
            // 驗證所有結果都在半徑內
            for result in &results {
                let actual_distance = result.dis.sqrt();
                assert!(
                    actual_distance <= radius,
                    "找到的實體應該在搜尋半徑內: {} <= {}",
                    actual_distance,
                    radius
                );
            }
            
            // 測試案例2：大範圍搜尋
            let search_pos = Vec2::new(4.0, 4.0);
            let radius = 10.0;
            let max_results = 20;
            let results = posdata.SearchNN_XY(search_pos, radius, max_results);
            
            // 驗證結果按距離排序
            for i in 1..results.len() {
                assert!(
                    results[i-1].dis <= results[i].dis,
                    "結果應該按距離升序排列"
                );
            }
        }
        
        /// 測試 SearchNN_XY2 函數 - 使用雙半徑搜尋
        #[test]
        fn test_search_nn_xy2() {
            let mut posdata = PosData::new();
            let (entities, positions) = create_test_data();
            
            // 建立索引
            for (entity, pos) in entities.iter().zip(positions.iter()) {
                posdata.xpos.push(PosXIndex { e: *entity, p: *pos });
                posdata.ypos.push(PosYIndex { e: *entity, p: *pos });
            }
            
            // 排序索引
            posdata.xpos.sort_by(|a, b| a.p.x.partial_cmp(&b.p.x).unwrap());
            posdata.ypos.sort_by(|a, b| a.p.y.partial_cmp(&b.p.y).unwrap());
            
            // 測試案例：使用內外兩個半徑
            let search_pos = Vec2::new(4.0, 4.0);
            let inner_radius = 3.0;
            let outer_radius = 6.0;
            let max_results = 10;
            
            let (inner_results, outer_results) = 
                posdata.SearchNN_XY2(search_pos, inner_radius, outer_radius, max_results);
            
            // 驗證內圈結果
            assert!(inner_results.len() <= max_results, "內圈結果不應超過最大值");
            for result in &inner_results {
                let actual_distance = result.dis.sqrt();
                assert!(
                    actual_distance <= inner_radius,
                    "內圈實體應該在內圈半徑內"
                );
            }
            
            // 驗證外圈結果
            for result in &outer_results {
                let actual_distance = result.dis.sqrt();
                assert!(
                    actual_distance >= inner_radius,
                    "外圈實體應該在內圈半徑外"
                );
                assert!(
                    actual_distance <= outer_radius,
                    "外圈實體應該在外圈半徑內"
                );
            }
            
            // 驗證內圈結果按距離排序
            for i in 1..inner_results.len() {
                assert!(
                    inner_results[i-1].dis <= inner_results[i].dis,
                    "內圈結果應該按距離升序排列"
                );
            }
        }
        
        /// 測試邊界情況 - 空資料
        #[test]
        fn test_empty_search() {
            let posdata = PosData::new();
            
            let search_pos = Vec2::new(0.0, 0.0);
            let radius = 10.0;
            let max_results = 5;
            
            // 測試所有搜尋函數在空資料時的行為
            let results_x = posdata.SearchNN_X(search_pos, radius, max_results);
            assert_eq!(results_x.len(), 0, "空資料應該返回空結果");
            
            let results_xy = posdata.SearchNN_XY(search_pos, radius, max_results);
            assert_eq!(results_xy.len(), 0, "空資料應該返回空結果");
            
            let (inner, outer) = posdata.SearchNN_XY2(search_pos, radius/2.0, radius, max_results);
            assert_eq!(inner.len(), 0, "空資料應該返回空內圈結果");
            assert_eq!(outer.len(), 0, "空資料應該返回空外圈結果");
        }
        
        /// 測試邊界情況 - 單一實體
        #[test]
        fn test_single_entity() {
            let mut posdata = PosData::new();
            let mut world = World::new();
            
            let entity = world.create_entity().build();
            let pos = Vec2::new(5.0, 5.0);
            
            posdata.xpos.push(PosXIndex { e: entity, p: pos });
            posdata.ypos.push(PosYIndex { e: entity, p: pos });
            
            // 測試找到單一實體
            let search_pos = Vec2::new(5.0, 5.0);
            let radius = 1.0;
            let max_results = 10;
            
            let results = posdata.SearchNN_X(search_pos, radius, max_results);
            assert_eq!(results.len(), 1, "應該找到唯一的實體");
            assert_eq!(results[0].e, entity, "應該找到正確的實體");
            assert_eq!(results[0].dis, 0.0, "距離應該為0");
            
            // 測試找不到實體（超出範圍）
            let search_pos = Vec2::new(10.0, 10.0);
            let radius = 1.0;
            let results = posdata.SearchNN_X(search_pos, radius, max_results);
            assert_eq!(results.len(), 0, "不應該找到任何實體");
        }
        
        /// 性能測試 - 大量資料
        #[test]
        fn test_performance_large_dataset() {
            let mut posdata = PosData::new();
            let mut world = World::new();
            
            // 建立大量測試資料 (100x100 網格)
            for x in 0..100 {
                for y in 0..100 {
                    let entity = world.create_entity().build();
                    let pos = Vec2::new(x as f32, y as f32);
                    posdata.xpos.push(PosXIndex { e: entity, p: pos });
                    posdata.ypos.push(PosYIndex { e: entity, p: pos });
                }
            }
            
            // 排序索引
            posdata.xpos.sort_by(|a, b| a.p.x.partial_cmp(&b.p.x).unwrap());
            posdata.ypos.sort_by(|a, b| a.p.y.partial_cmp(&b.p.y).unwrap());
            
            // 測試搜尋性能
            let search_pos = Vec2::new(50.0, 50.0);
            let radius = 10.0;
            let max_results = 100;
            
            let start = std::time::Instant::now();
            let results = posdata.SearchNN_XY(search_pos, radius, max_results);
            let duration = start.elapsed();
            
            // 驗證結果
            assert!(results.len() > 0, "應該找到實體");
            assert!(results.len() <= max_results, "結果不應超過最大值");
            
            // 確保搜尋時間合理（應該在毫秒級別）
            assert!(
                duration.as_millis() < 100,
                "搜尋應該在100毫秒內完成，實際耗時: {:?}",
                duration
            );
        }
    }
}
