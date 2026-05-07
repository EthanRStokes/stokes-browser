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
mod shell_provider;
mod default_browser;

use cosmic::app::Settings;
use tokio::runtime::Builder;

fn parse_scale_factor(var: &str) -> Option<f32> {
    std::env::var(var)
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| *value > 0.1)
}

fn resolve_scale_factor() -> Option<f32> {
    let env_candidates = [
        "COSMIC_SCALE",
        "GDK_SCALE",
        "QT_SCALE_FACTOR",
        "WINIT_SCALE_FACTOR",
        "WINIT_HIDPI_FACTOR",
    ];

    for var in env_candidates {
        if let Some(value) = parse_scale_factor(var) {
            return Some(value);
        }
    }

    std::env::var("XFT_DPI")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .map(|dpi| (dpi / 96.0).max(0.1))
}

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

    let use_legacy = args.iter().any(|a| a == "--legacy");

    if use_legacy {
        return ui::legacy::browser::run(startup_url);
    }

    let mut settings = Settings::default();
    if let Some(scale_factor) = resolve_scale_factor() {
        settings = settings.scale_factor(scale_factor);
    }

    cosmic::app::run::<ui::libcosmic::CosmicBrowserApp>(settings, startup_url)
        .expect("cosmic run failed");

    Ok(())
}