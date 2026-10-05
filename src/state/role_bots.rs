//! Immutable authority-local controllers; not session authorization.
use omoba_core::runtime::native::moba_match::bots::{role_bot_inputs,RoleBotConfig};
use omoba_core::runtime::{MobaMatch,PlayerInput};
use specs::{World,WorldExt};
use std::collections::BTreeMap;

pub(crate) struct ServerRoleBotControllers {
    config:RoleBotConfig,
    human_teams:BTreeMap<u32,u32>,
    match_teams:BTreeMap<u32,u32>,
}

impl ServerRoleBotControllers {
    pub(crate) fn new(world:&World,config:RoleBotConfig,human_teams:BTreeMap<u32,u32>) -> Result<Self,String> {
        let state=world.try_fetch::<MobaMatch>().ok_or("role bots require a MOBA match")?;
        config.validate(&state).map_err(str::to_owned)?;
        let match_teams:BTreeMap<_,_>=state.heroes.iter().map(|slot|(slot.player_id,state.config.teams[slot.side])).collect();
        let expected_humans:BTreeMap<_,_>=match_teams.iter()
            .filter(|(player,_)|!config.assignments.iter().any(|bot|bot.player_id==**player))
            .map(|(&player,&team)|(player,team)).collect();
        if expected_humans!=human_teams {
            return Err("human authentication and bot controllers must partition the match roster".into());
        }
        Ok(Self {config,human_teams,match_teams})
    }

    pub(crate) fn team(&self,player:u32) -> Option<u32> {self.match_teams.get(&player).copied()}

    pub(crate) fn merge_inputs(&self,world:&World,mut external:Vec<(u32,PlayerInput,u32)>)
        -> Result<Vec<(u32,PlayerInput,u32)>,String> {
        // Even a trusted host channel cannot silently seize a bot controller.
        external.retain(|(player,_,_)|self.human_teams.contains_key(player));
        external.extend(role_bot_inputs(world,&self.config).map_err(str::to_owned)?
            .into_iter().map(|(player,input)|(player,input,0)));
        // Stable sort retains a human's original same-player input ordering.
        external.sort_by_key(|(player,_,_)|*player);
        Ok(external)
    }
}
