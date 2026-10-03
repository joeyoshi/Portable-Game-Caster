mod discovery;
mod player;
mod state;
mod ui;

use state::AppState;
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Portable Game Caster - macOS Launcher");
    println!("-------------------------------------");

    ui::set_state(&AppState::Idle);

    ui::set_state(&AppState::Discovering);

    let endpoint = match discovery::discover_stream() {
        Ok(endpoint) => endpoint,

        Err(error) => {
            ui::set_state(&AppState::Error(error.to_string()));
            return Err(error);
        }
    };

    ui::set_state(&AppState::Resolving);

    println!("Found Portable Game Caster:");
    println!("  Host: {}", endpoint.host);
    println!("  Address: {}", endpoint.address);
    println!("  Port: {}", endpoint.port);
    println!("  Path: {}", endpoint.path);
    println!("  URL: {}", endpoint.url());

    ui::set_state(&AppState::Connecting);

    let mut player = match player::launch_ffplay(&endpoint.url()) {
        Ok(player) => player,

        Err(error) => {
            ui::set_state(&AppState::Error(error.to_string()));
            return Err(error);
        }
    };

    ui::set_state(&AppState::WaitingForStream);

    // Temporary placeholder until we can determine stream readiness properly.
    thread::sleep(Duration::from_secs(1));

    ui::set_state(&AppState::Playing);

    let status = player.wait()?;

    if !status.success() {
        ui::set_state(&AppState::Error(format!(
            "ffplay exited with status: {}",
            status
        )));
    } else {
        ui::set_state(&AppState::Idle);
    }

    Ok(())
}