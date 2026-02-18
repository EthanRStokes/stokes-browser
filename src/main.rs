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
mod browser_iced;

use crate::browser::BrowserApp;
use winit::event_loop::EventLoop;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check if this is a tab process
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "--tab-process" {
        let tab_id = args[2].clone();
        let socket_path = std::path::PathBuf::from(&args[3]);
        let rt = tokio::runtime::Runtime::new()?;
        return rt.block_on(async {
            tab_process::tab_process_main(tab_id, socket_path).await.map_err(|e| e.into())
        });
    }

    // Check if we should run the iced version
    if args.iter().any(|arg| arg == "--iced") {
        println!("Starting Web Browser (Iced UI)...");
        return browser_iced::run_iced_browser().map_err(|e| e.into());
    }

    let rt = tokio::runtime::Runtime::new()?;

    // Main browser process
    println!("Starting Web Browser...");
    let event_loop = EventLoop::new()?;
    rt.block_on(async {
        let mut app = BrowserApp::new(&event_loop).await;

        app.add_tab();

        event_loop.run_app(&mut app)?;
        Ok(())
    })
}