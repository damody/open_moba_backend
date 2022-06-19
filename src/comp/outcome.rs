use crate::{comp, Creep, CProperty, TProperty};
use super::Projectile;
use hashbrown::HashSet;
use serde::{Deserialize, Serialize};
use vek::*;
use specs::Entity;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::ops::DerefMut;
use std::cmp::Ordering;
use voracious_radix_sort::{Radixable, RadixSort};
use crate::Tower;
use crate::TAttack;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Outcome {
    Damage {
        pos: Vec2<f32>,
        phys: f32,
        magi: f32,
        real: f32,
        source: Entity,
        target: Entity,
    },
    ProjectileLine2 {
        pos: Vec2<f32>,
        source: Option<Entity>,
        target: Option<Entity>,
    },
    Death {
        pos: Vec2<f32>,
        ent: Entity,
    },
    Creep {
        cd: CreepData,
    },
    CreepStop {
        source: Entity,
        target: Entity,
    },
    CreepWalk {
        target: Entity,
    },
    Tower {
        pos: Vec2<f32>,
        td: TowerData,
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreepData {
    pub pos: Vec2<f32>,
    pub creep: Creep,
    pub cdata: CProperty,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TowerData {
    pub tpty: TProperty,
    pub tatk: TAttack,
}

#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct PosXIndex {
    pub e: Entity,
    pub p: Vec2<f32>,
}
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct PosYIndex {
    pub e: Entity,
    pub p: Vec2<f32>,
}
impl PartialOrd for PosXIndex {
    fn partial_cmp(&self, other: &PosXIndex) -> Option<Ordering> {
        self.p.x.partial_cmp(&other.p.x)
    }
}
impl PartialEq for PosXIndex {
    fn eq(&self, other: &Self) -> bool {
        self.p.x == other.p.x
    }
}
impl Radixable<f32> for PosXIndex {
    type Key = f32;
    #[inline]
    fn key(&self) -> Self::Key {
        self.p.x
    }
}
impl PartialOrd for PosYIndex {
    fn partial_cmp(&self, other: &PosYIndex) -> Option<Ordering> {
        self.p.y.partial_cmp(&other.p.y)
    }
}
impl PartialEq for PosYIndex {
    fn eq(&self, other: &Self) -> bool {
        self.p.y == other.p.y
    }
}
impl Radixable<f32> for PosYIndex {
    type Key = f32;
    #[inline]
    fn key(&self) -> Self::Key {
        self.p.y
    }
}
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct DisIndex {
    pub e: Entity,
    pub dis: f32,
}
impl Eq for DisIndex {}
impl Ord for DisIndex {
    fn cmp(&self, other: &Self) -> Ordering{
        self.dis.partial_cmp(&other.dis).unwrap()
    }
}
impl PartialOrd for DisIndex {
    fn partial_cmp(&self, other: &DisIndex) -> Option<Ordering> {
        self.dis.partial_cmp(&other.dis)
    }
}
impl PartialEq for DisIndex {
    fn eq(&self, other: &Self) -> bool {
        self.dis == other.dis
    }
}
impl Radixable<f32> for DisIndex {
    type Key = f32;
    #[inline]
    fn key(&self) -> Self::Key {
        self.dis
    }
}

#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct DisIndex2 {
    pub e: Entity,
    pub p: Vec2<f32>,
}
impl Eq for DisIndex2 {}
impl Ord for DisIndex2 {
    fn cmp(&self, other: &Self) -> Ordering{
        self.e.cmp(&other.e)
    }
}
impl PartialOrd for DisIndex2 {
    fn partial_cmp(&self, other: &DisIndex2) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for DisIndex2 {
    fn eq(&self, other: &Self) -> bool {
        self.e == other.e
    }
}
impl Radixable<u32> for DisIndex2 {
    type Key = u32;
    #[inline]
    fn key(&self) -> Self::Key {
        self.e.id()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Searcher {
    pub tower: PosData,
    pub creep: PosData,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PosData {
    pub xpos: Vec<PosXIndex>,
    pub ypos: Vec<PosYIndex>,
    pub needsort: bool,
}
impl PosData {
    pub fn new() -> PosData {
        PosData {
            xpos: vec![],
            ypos: vec![],
            needsort: false,
        }
    }
    
    pub fn SearchNN_XY2(&self, pos: Vec2<f32>, radius1: f32, radius2: f32, n: usize) -> (Vec<DisIndex>, Vec<DisIndex>) {
        let r2 = radius1*radius1;
        let mut res = vec![];
        let mut res2 = vec![];
        let mut xdata = vec![];
        let mut ydata = vec![];
        let lx = pos.x - radius2;
        let rx = pos.x + radius2;
        let lxp = self.xpos.binary_search_by(|data| data.p.x.partial_cmp(&lx).unwrap());
        let lxi = match lxp {
            Ok(x) => {x}
            Err(x) => {x}
        };
        let rxp = self.xpos.binary_search_by(|data| data.p.x.partial_cmp(&rx).unwrap());
        let rxi = match rxp {
            Ok(x) => {x}
            Err(x) => {x}
        };
        for i in lxi..rxi {
            if let Some(p) = self.ypos.get(i) {
                xdata.push(DisIndex2 { e: p.e, p: p.p });
            }
        }
        let ly = pos.y - radius2;
        let ry = pos.y + radius2;
        let lyp = self.ypos.binary_search_by(|data| data.p.y.partial_cmp(&ly).unwrap());
        let lyi = match lyp {
            Ok(y) => {y}
            Err(y) => {y}
        };
        let ryp = self.ypos.binary_search_by(|data| data.p.y.partial_cmp(&ry).unwrap());
        let ryi = match ryp {
            Ok(y) => {y}
            Err(y) => {y}
        };
        for i in lyi..ryi {
            if let Some(p) = self.ypos.get(i) {
                ydata.push(DisIndex2 { e: p.e, p: p.p });
            }
        }
        xdata.voracious_sort();
        ydata.voracious_sort();
        let mut ary = [xdata.iter(), ydata.iter()];
        let intersection_iter = 
            sorted_intersection::SortedIntersection::new(&mut ary);
        for p in intersection_iter {
            let dis = p.p.distance_squared(pos);
            if dis < r2 {
                res.push(DisIndex { e: p.e, dis: dis });
            } else {
                res2.push(DisIndex { e: p.e, dis: dis });
            }
        }
        res.voracious_sort();
        res.truncate(n);
        (res, res2)
    }
    pub fn SearchNN_XY(&self, pos: Vec2<f32>, radius: f32, n: usize) -> Vec<DisIndex> {
        let r2 = radius*radius;
        let mut res = vec![];
        let mut xdata = vec![];
        let mut ydata = vec![];
        let xp = self.xpos.binary_search_by(|data| data.p.x.partial_cmp(&pos.x).unwrap());
        let xidx = match xp {
            Ok(x) => {x}
            Err(x) => {x}
        };
        let mut loffset = 0;
        let mut roffset = 1;
        loop {
            if let Some(p) = self.xpos.get((xidx as i32 - loffset) as usize) {
                if (p.p.x - pos.x).abs() < radius {
                    xdata.push(DisIndex2 { e: p.e, p: p.p });
                } else {
                    break;
                }
            } else {
                break;
            }
            loffset += 1;
        }
        loop {
            if let Some(p) = self.xpos.get((xidx as i32 + roffset) as usize) {
                if (p.p.x + pos.x).abs() < radius {
                    xdata.push(DisIndex2 { e: p.e, p: p.p });
                } else {
                    break;
                }
            } else {
                break;
            }
            roffset += 1;
        }
        let yp = self.ypos.binary_search_by(|data| data.p.y.partial_cmp(&pos.y).unwrap());
        let yidx = match yp {
            Ok(y) => {y}
            Err(y) => {y}
        };
        let mut loffset = 0;
        let mut roffset = 1;
        loop {
            if let Some(p) = self.ypos.get((yidx as i32 - loffset) as usize) {
                if (p.p.y - pos.y).abs() < radius {
                    ydata.push(DisIndex2 { e: p.e, p: p.p });
                } else {
                    break;
                }
            } else {
                break;
            }
            loffset += 1;
        }
        loop {
            if let Some(p) = self.ypos.get((yidx as i32 + roffset) as usize) {
                if (p.p.y + pos.y).abs() < radius {
                    ydata.push(DisIndex2 { e: p.e, p: p.p });
                } else {
                    break;
                }
            } else {
                break;
            }
            roffset += 1;
        }
        xdata.voracious_sort();
        ydata.voracious_sort();
        let mut ary = [xdata.iter(), ydata.iter()];
        let intersection_iter = 
            sorted_intersection::SortedIntersection::new(&mut ary);
        for p in intersection_iter {
            let dis = p.p.distance_squared(pos);
            if dis < r2 {
                res.push(DisIndex { e: p.e, dis: dis });
            }
        }
        res.voracious_sort();
        res.truncate(n);
        res
    }
    pub fn SearchNN_X(&self, pos: Vec2<f32>, radius: f32, n: usize) -> Vec<DisIndex> {
        let r2 = radius*radius;
        let mut res = vec![];
        let xp = self.xpos.binary_search_by(|data| data.p.x.partial_cmp(&pos.x).unwrap());
        let xidx = match xp {
            Ok(x) => {
                x
            }
            Err(x) => {
                x
            }
        };
        let mut loffset = 0;
        let mut roffset = 1;
        loop {
            if let Some(p) = self.xpos.get((xidx as i32 - loffset) as usize) {
                if (p.p.x - pos.x).abs() < radius {
                    let dis = p.p.distance_squared(pos);
                    if dis < r2 {
                        res.push(DisIndex { e: p.e, dis: dis });
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
            loffset += 1;
        }
        loop {
            if let Some(p) = self.xpos.get((xidx as i32 + roffset) as usize) {
                if (p.p.x - pos.x).abs() < radius {
                    let dis = p.p.distance_squared(pos);
                    if dis < r2 {
                        res.push(DisIndex { e: p.e, dis: dis });
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
            roffset += 1;
        }
        res.sort_unstable();
        res.truncate(n);
        res
    }
}
