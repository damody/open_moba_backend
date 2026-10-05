//! Configuration-only preflight: no CONFIG singleton, DLL, World or sockets.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 || args[0] != "--config" {
        return Err("usage: moba-config --config FILE.toml".into());
    }
    let text = std::fs::read_to_string(&args[1])?;
    let report = omobab::config::server_config::validate_moba_launch_configuration(&text)?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
