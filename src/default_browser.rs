/// Sets Stokes Browser as the system default browser.
///
/// This is a best-effort operation. Failures are logged but not fatal.
pub fn set_as_default_browser() {
    #[cfg(target_os = "linux")]
    set_default_linux();

    #[cfg(target_os = "macos")]
    set_default_macos();

    #[cfg(target_os = "windows")]
    set_default_windows();
}

// ─────────────────────────────────────────────────────────────────────────────
// Linux
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn set_default_linux() {
    use std::process::Command;

    // Set the default web-browser via xdg-settings (works on most freedesktop DEs)
    let status = Command::new("xdg-settings")
        .args(["set", "default-web-browser", "com.ethanstokes.stokes-browser.desktop"])
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("[default_browser] Set as default browser via xdg-settings");
        }
        Ok(s) => {
            eprintln!("[default_browser] xdg-settings exited with status {}", s);
            // Fallback: update-desktop-database / xdg-mime
            set_default_linux_xdg_mime();
        }
        Err(e) => {
            eprintln!("[default_browser] xdg-settings not found: {}. Trying xdg-mime fallback.", e);
            set_default_linux_xdg_mime();
        }
    }
}

#[cfg(target_os = "linux")]
fn set_default_linux_xdg_mime() {
    use std::process::Command;

    let desktop = "com.ethanstokes.stokes-browser.desktop";
    let mime_types = ["x-scheme-handler/http", "x-scheme-handler/https", "text/html"];

    for mime in &mime_types {
        let status = Command::new("xdg-mime")
            .args(["default", desktop, mime])
            .status();
        match status {
            Ok(s) if s.success() => {
                println!("[default_browser] xdg-mime default set for {}", mime);
            }
            Ok(s) => eprintln!("[default_browser] xdg-mime failed for {}: {}", mime, s),
            Err(e) => eprintln!("[default_browser] xdg-mime not found for {}: {}", mime, e),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// macOS
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn set_default_macos() {
    // On macOS the recommended way to set the default browser without a GUI
    // prompt is via the `defaultbrowser` CLI (homebrew) or the LaunchServices
    // private API.  We try three approaches in order:
    //   1. `defaultbrowser` CLI tool
    //   2. AppleScript (prompts the user but always works)
    //   3. Write LSHandlers in ~/Library/Preferences/com.apple.LaunchServices/...
    //      (requires a re-login to take effect)
    //
    // The bundle identifier must match what is declared in the app's Info.plist.
    let bundle_id = "com.ethanstokes.stokes-browser";

    if try_defaultbrowser_cli(bundle_id) {
        return;
    }

    try_applescript_default_browser();
}

#[cfg(target_os = "macos")]
fn try_defaultbrowser_cli(bundle_id: &str) -> bool {
    use std::process::Command;

    // `defaultbrowser` is a small open-source tool:
    //   https://github.com/kerma/defaultbrowser
    // The app name it expects is the bundle ID lowercased without the domain.
    let short_name = bundle_id.split('.').last().unwrap_or(bundle_id);

    let status = Command::new("defaultbrowser")
        .arg(short_name)
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("[default_browser] Set default browser via 'defaultbrowser' CLI");
            true
        }
        _ => false,
    }
}

#[cfg(target_os = "macos")]
fn try_applescript_default_browser() {
    use std::process::Command;

    // This will show the macOS "Change Default Browser?" dialog.
    let script = r#"tell application "Stokes Browser" to activate"#;

    // A more direct approach: use LSSetDefaultHandlerForURLScheme via osascript
    // Since we can't link CoreServices easily here without a build script, we
    // fall back to writing the LaunchServices plist directly.
    let status = Command::new("osascript")
        .args(["-e", script])
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("[default_browser] Activated app via osascript");
        }
        _ => {}
    }

    // Write LSHandlers to the LaunchServices plist (takes effect after relogin)
    write_launchservices_plist();
}

#[cfg(target_os = "macos")]
fn write_launchservices_plist() {
    use std::process::Command;

    let bundle_id = "com.ethanstokes.stokes-browser";

    for scheme in &["http", "https"] {
        // Use `defaults write` to set the handler in the LaunchServices database
        let _ = Command::new("defaults")
            .args([
                "write",
                "com.apple.LaunchServices/com.apple.launchservices.secure",
                "LSHandlers",
                "-array-add",
                &format!(
                    "{{LSHandlerURLScheme = {}; LSHandlerRoleAll = {}; }}",
                    scheme, bundle_id
                ),
            ])
            .status();
    }

    // Rebuild the LS database
    let _ = Command::new("/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister")
        .args(["-kill", "-r", "-domain", "local", "-domain", "system", "-domain", "user"])
        .status();

    println!("[default_browser] Wrote LaunchServices plist entries (takes effect after re-login)");
}

// ─────────────────────────────────────────────────────────────────────────────
// Windows
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn set_default_windows() {
    use std::path::PathBuf;

    // Get the path to the current executable
    let exe_path = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[default_browser] Could not determine exe path: {}", e);
            return;
        }
    };

    if let Err(e) = register_app_windows(&exe_path) {
        eprintln!("[default_browser] Failed to register app in registry: {}", e);
        return;
    }

    // On Windows 8+, the user must confirm the default browser change through
    // the Settings UI.  We open the relevant settings page as a best effort.
    open_default_apps_settings();
}

#[cfg(target_os = "windows")]
fn register_app_windows(exe_path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use winreg::enums::*;
    use winreg::RegKey;

    let exe_str = exe_path.to_string_lossy();
    let app_name = "StokesBrowser";
    let prog_id = "StokesBrowser.HTML";

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // ── 1. Register ProgID ──────────────────────────────────────────────────
    // HKCU\Software\Classes\StokesBrowser.HTML
    let (prog_id_key, _) = hkcu.create_subkey(format!("Software\\Classes\\{}", prog_id))?;
    prog_id_key.set_value("", &"Stokes Browser HTML Document")?;

    let (icon_key, _) = prog_id_key.create_subkey("DefaultIcon")?;
    icon_key.set_value("", &format!("{},0", exe_str))?;

    let (open_key, _) = prog_id_key.create_subkey("shell\\open\\command")?;
    open_key.set_value("", &format!("\"{}\" \"%1\"", exe_str))?;

    // ── 2. Register the application ────────────────────────────────────────
    // HKCU\Software\Clients\StartMenuInternet\StokesBrowser
    let (clients_key, _) = hkcu.create_subkey(
        format!("Software\\Clients\\StartMenuInternet\\{}", app_name)
    )?;
    clients_key.set_value("", &"Stokes Browser")?;

    let (app_icon_key, _) = clients_key.create_subkey("DefaultIcon")?;
    app_icon_key.set_value("", &format!("{},0", exe_str))?;

    let (capabilities_key, _) = clients_key.create_subkey("Capabilities")?;
    capabilities_key.set_value("ApplicationName", &"Stokes Browser")?;
    capabilities_key.set_value("ApplicationDescription", &"Web browser developed by Ethan Stokes")?;

    let (url_assoc_key, _) = capabilities_key.create_subkey("URLAssociations")?;
    url_assoc_key.set_value("http", &prog_id)?;
    url_assoc_key.set_value("https", &prog_id)?;
    url_assoc_key.set_value("ftp", &prog_id)?;

    let (file_assoc_key, _) = capabilities_key.create_subkey("FileAssociations")?;
    file_assoc_key.set_value(".htm", &prog_id)?;
    file_assoc_key.set_value(".html", &prog_id)?;

    let (shell_key, _) = clients_key.create_subkey("shell\\open\\command")?;
    shell_key.set_value("", &format!("\"{}\"", exe_str))?;

    // ── 3. Register under RegisteredApplications ───────────────────────────
    let (reg_apps_key, _) = hkcu.create_subkey("Software\\RegisteredApplications")?;
    reg_apps_key.set_value(
        app_name,
        &format!(
            "Software\\Clients\\StartMenuInternet\\{}\\Capabilities",
            app_name
        ),
    )?;

    println!("[default_browser] Registered app in Windows registry");
    Ok(())
}

#[cfg(target_os = "windows")]
fn open_default_apps_settings() {
    use std::process::Command;

    // Open "Default Apps" in Windows Settings so the user can confirm
    let _ = Command::new("cmd")
        .args(["/c", "start", "ms-settings:defaultapps"])
        .status();

    println!("[default_browser] Opened Windows Default Apps settings page");
}

