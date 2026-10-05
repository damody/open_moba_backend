//! Configuration/selection entrypoints; neither mode creates a gameplay World or loads a DLL.
#[path = "moba_config/selection_network.rs"]
mod selection_network;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if (args.len() == 3 || args.len() == 4) && args[0] == "--selection-host" {
        let address = if args.len() == 4 {
            args[2]
                .to_str()
                .ok_or("bind address must be UTF-8")?
                .parse()?
        } else {
            "127.0.0.1:0".parse()?
        };
        let output = if args.len() == 4 { &args[3] } else { &args[2] };
        selection_network::host(&args[1], address, output)?;
        return Ok(());
    }
    if args.len() == 3 && args[0] == "--selection-session" {
        use omoba_core::runtime::native::moba_match::selection::{
            service::SelectionService, HeroSelectionSession,
        };
        let player_id = args[2].to_str().ok_or("player ID must be UTF-8")?.parse()?;
        let text = std::fs::read_to_string(&args[1])?;
        if selection_network::is_invitation(&text)? {
            selection_network::proxy(&text, player_id)?;
            return Ok(());
        }
        let plan = serde_json::from_str(&text)?;
        let mut service =
            SelectionService::new(HeroSelectionSession::new(plan, 0, 60)?, player_id)?;
        service.serve(std::io::stdin().lock(), std::io::stdout().lock())?;
        return Ok(());
    }
    if args.len() != 2 || (args[0] != "--config" && args[0] != "--lock-plan") {
        return Err("usage: moba-config --config FILE.toml | --lock-plan FILE.json | --selection-session PLAN_OR_INVITE.json ADMITTED_PLAYER_ID | --selection-host PLAN.json [BIND_IP:PORT] NEW_OUTPUT_DIRECTORY".into());
    }
    let text = std::fs::read_to_string(&args[1])?;
    if args[0] == "--lock-plan" {
        use omoba_core::runtime::native::moba_match::{
            bots::RoleBotMatchPlan,
            selection::{HeroSelectionAction, HeroSelectionRequest, HeroSelectionSession},
        };
        let plan: RoleBotMatchPlan = serde_json::from_str(&text)?;
        let humans: Vec<_> = plan
            .players
            .iter()
            .filter(|p| !p.bot)
            .map(|p| p.player_id)
            .collect();
        let mut selection = HeroSelectionSession::new(plan, 0, 60)?;
        // Trusted local host preparation, not remote player consent. Network UI
        // must call the same kernel with an admitted identity for each request.
        for player_id in humans {
            selection.apply(
                player_id,
                HeroSelectionRequest {
                    expected_revision: selection.snapshot().revision,
                    action: HeroSelectionAction::Lock,
                },
            )?;
        }
        let plan = selection.finalize(selection.snapshot().revision)?;
        println!(
            "{}",
            serde_json::json!({"schema_version":1,"scope":"host-prepared-selection",
            "tick_rate_hz":60,"selection":selection.snapshot(),"plan":plan,
            "hero_catalog":omoba_core::runtime::native::moba_match::selection::hero_selection_catalog()})
        );
        return Ok(());
    }
    let report = omobab::config::server_config::validate_moba_launch_configuration(&text)?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
