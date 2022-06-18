use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CreepWaveData {
    pub Path: Vec<PathJD>,
    pub Creep: Vec<CreepJD>,
    pub CheckPoint: Vec<CheckPointJD>,
    pub Tower: Vec<TowerJD>,
    pub CreepWave: Vec<CreepWaveJD>,    
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PathJD {
    pub Name: String,
    pub Points: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CreepJD {
    pub Name: String,
    pub HP: f32,
    pub DefendPhysic: f32,
    pub DefendMagic: f32,
    pub MoveSpeed: f32,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CheckPointJD {
    pub Name: String,
    pub Class: String,
    pub X: f32,
    pub Y: f32,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TowerJD {
    pub Name: String,
    pub Property: PropertyJD,
    pub Attack: AttackJD,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AttackJD {
    pub Range: f32,
    pub AttackSpeed: f32,
    pub Physic: f32,
    pub Magic: f32,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PropertyJD {
    pub Hp: i32,
    pub Block: i32,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CreepWaveJD {
    pub Name: String,
    pub StartTime: f32,
    pub Detail: Vec<DetailJD>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct DetailJD {
    pub Path: String,
    pub Creeps: Vec<CreepsJD>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CreepsJD {
    pub Time: f32,
    pub Creep: String,
}

