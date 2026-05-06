#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod engine;
mod networking;
mod ui;
mod dom;
mod layout;
mod renderer;
mod css;
mod js;
pub mod convert_events;
pub mod events;
mod ipc;
mod tab_process;
mod tab_manager;
mod cosmic_app;
mod shell_provider;
mod default_browser;
mod bookmarks;

use cosmic::app::Settings;
use tokio::runtime::Builder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check if this is a tab process
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "--tab-process" {
        let tab_id = args[2].clone();
        let server_name = args[3].clone();

        // tokio
        let rt = Builder::new_multi_thread()
            .enable_all()
            .build()?;

        return rt.block_on(async {
            tab_process::tab_process_main(tab_id, server_name).await.map_err(|e| e.into())
        });
    }

    // Main browser process
    println!("Starting Stokes Browser...");

    // Check for a URL passed as a command-line argument (e.g. when launched as the default browser)
    let startup_url: Option<String> = args.iter().skip(1).find(|a| {
        a.starts_with("http://") || a.starts_with("https://") || a.starts_with("about:")
    }).cloned();

    cosmic::app::run::<cosmic_app::CosmicBrowserApp>(Settings::default(), startup_url)
        .expect("cosmic run failed");

    Ok(())
}