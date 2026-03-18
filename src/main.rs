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
mod input;
mod ipc;
mod tab_process;
mod tab_manager;
mod browser;
mod window;
mod shell_provider;
mod default_browser;

use crate::browser::BrowserApp;
use winit::event_loop::EventLoop;
use winit_core::event_loop::ControlFlow;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check if this is a tab process
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "--tab-process" {
        let tab_id = args[2].clone();
        let server_name = args[3].clone();
        return tab_process::tab_process_main(tab_id, server_name).await.map_err(|e| e.into());
    }

    // Main browser process
    println!("Starting Stokes Browser...");

    // Check for a URL passed as a command-line argument (e.g. when launched as the default browser)
    let startup_url: Option<String> = args.iter().skip(1).find(|a| {
        a.starts_with("http://") || a.starts_with("https://") || a.starts_with("about:")
    }).cloned();
    for arg in args {
        println!("{}", arg);
    }

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let app = BrowserApp::new(&event_loop, startup_url).await;

    event_loop.run_app(app)?;
    Ok(())
}