use erps_proto::v1 as pb;

#[derive(Clone, Debug)]
pub struct LaunchAssignment { pub match_id:String, pub mode:i32, pub teams:Vec<Vec<String>>, pub reserved_cost:u32 }
impl From<pb::LaunchMatch> for LaunchAssignment { fn from(v:pb::LaunchMatch)->Self{Self{match_id:v.match_id,mode:v.mode,teams:v.teams.into_iter().map(|t|t.player_ids).collect(),reserved_cost:v.reserved_cost}} }
#[derive(Clone, Debug)]
pub struct InstanceReport { pub match_id:String, pub state:String, pub reserved_cost:u32 }
#[derive(Clone, Debug)]
pub struct LaunchReady { pub endpoint:String, pub connection_token:String }

/// Bridges ERPS lifecycle messages to the game's own instance manager.
pub trait GameInstanceAdapter:Send+Sync+'static {
    fn launch(&self, assignment:LaunchAssignment)->Result<LaunchReady,String>;
    fn snapshot(&self)->Vec<InstanceReport>;
}
