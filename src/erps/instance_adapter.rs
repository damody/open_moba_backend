use erps_proto::v1 as pb;

#[derive(Clone, Debug)]
pub struct LaunchAssignment {
    pub match_id: String,
    pub mode: i32,
    pub teams: Vec<Vec<String>>,
    pub reserved_cost: u32,
}
impl From<pb::LaunchMatch> for LaunchAssignment {
    fn from(v: pb::LaunchMatch) -> Self {
        Self {
            match_id: v.match_id,
            mode: v.mode,
            teams: v.teams.into_iter().map(|t| t.player_ids).collect(),
            reserved_cost: v.reserved_cost,
        }
    }
}
#[derive(Clone, Debug)]
pub struct InstanceReport {
    pub match_id: String,
    pub state: String,
    pub reserved_cost: u32,
    /// Required for Ready/Running reconciliation so a lost ready event can be replayed.
    pub endpoint: String,
    /// Required for Ready/Running reconciliation; treat as a credential and never log it.
    pub connection_token: String,
}
#[derive(Clone, Debug)]
pub struct LaunchReady {
    pub endpoint: String,
    pub connection_token: String,
}
#[derive(Clone, Debug)]
pub struct MatchCompletion {
    pub match_id: String,
    pub placements: Vec<(String, u32)>,
}

/// Bridges ERPS lifecycle messages to the game's own instance manager.
pub trait GameInstanceAdapter: Send + Sync + 'static {
    fn launch(&self, assignment: LaunchAssignment) -> Result<LaunchReady, String>;
    fn snapshot(&self) -> Vec<InstanceReport>;
    /// Returns one newly finished outcome, or `None`. Every roster player must appear exactly
    /// once. The client stops polling when its bounded reliable outbox is full.
    fn poll_completed_result(&self) -> Option<MatchCompletion> {
        None
    }
}
