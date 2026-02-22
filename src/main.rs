mod engine;
mod networking;
mod ui;
mod dom;
mod layout;
mod renderer;
mod css;
mod js;
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check if this is a tab process
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "--tab-process" {
        let tab_id = args[2].clone();
        let socket_path = std::path::PathBuf::from(&args[3]);
        return tab_process::tab_process_main(tab_id, socket_path).await.map_err(|e| e.into());
    }

    // Main browser process
    println!("Starting Web Browser...");

    // Attempt to register this app as the default browser
    default_browser::set_as_default_browser();
    let event_loop = EventLoop::new()?;
    let mut app = BrowserApp::new(&event_loop).await;

    // Create initial tab
    app.add_tab();

    event_loop.run_app(&mut app)?;
    Ok(())
}