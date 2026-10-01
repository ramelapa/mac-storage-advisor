//! Native window around the same localhost page.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use clap::Args;
use mac_storage_common::PRODUCT_NAME;
use mac_storage_storage::{resolve_db_path, Database};

use crate::{ui, Error};

const DEFAULT_PORT: u16 = 47231;

#[derive(Debug, Args)]
pub struct WindowArgs {
    /// Port for the local page. The window loads 127.0.0.1 on this port.
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,

    /// SQLite path. Overrides the MAC_STORAGE_DB environment variable.
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,
}

impl WindowArgs {
    /// Defaults used when Finder opens the Mac app. The database stays the usual file.
    pub(crate) fn for_app_launch() -> Self {
        Self {
            port: DEFAULT_PORT,
            db: None,
        }
    }
}

pub fn window_command(args: WindowArgs) -> Result<(), Error> {
    if args.port == 0 {
        return Err(Error::Usage("--port must be between 1 and 65535".into()));
    }
    let database =
        resolve_db_path(args.db.as_deref()).map_err(|err| Error::Storage(err.to_string()))?;
    Database::open(&database).map_err(|err| Error::Storage(err.to_string()))?;
    let stop = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::channel();
    let server_stop = Arc::clone(&stop);
    let server_db = database.clone();
    let server = thread::spawn(move || ui::serve(server_db, args.port, sender, server_stop, false));
    let listen = match receiver.recv_timeout(Duration::from_secs(5)) {
        Ok(addr) => addr,
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            let _ = server.join();
            return Err(Error::Io(std::io::Error::other(
                "the local page did not start",
            )));
        }
    };
    let page = window_url(listen);
    println!(
        "{PRODUCT_NAME}\n\
         Window: {page}\n\
         Database: {}\n\
         Close the window to stop. The page stays on this computer.",
        database.display()
    );
    let opened = open_window(&page);
    stop.store(true, Ordering::Relaxed);
    let _ = server.join();
    opened
}

pub(crate) fn window_url(addr: SocketAddr) -> String {
    format!("http://{addr}")
}

/// True when this process is the Mac app opened with no advisor command.
///
/// Finder passes the executable path and, on older systems, a `-psn_` argument.
/// `Mac Storage Advisor.app/Contents/MacOS/mac-storage scan ~/Downloads` stays a command.
pub(crate) fn launched_as_mac_app(args: &[String], executable: Option<&str>) -> bool {
    let exe = match executable {
        Some(path) if !path.is_empty() => path,
        _ => match args.first() {
            Some(arg) => arg.as_str(),
            None => return false,
        },
    };
    if !executable_is_in_mac_app(exe) {
        return false;
    }
    args.iter().skip(1).all(|arg| arg.starts_with("-psn_"))
}

fn executable_is_in_mac_app(exe: &str) -> bool {
    let parts: Vec<&str> = exe
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    parts.windows(3).any(|window| {
        window[0].ends_with(".app")
            && window[0].len() > ".app".len()
            && window[1] == "Contents"
            && window[2] == "MacOS"
    })
}

pub(crate) fn app_launch_dialog_script(message: &str) -> String {
    let message: String = message.chars().take(400).collect();
    format!(
        "display dialog {message} buttons {{\"OK\"}} default button \"OK\" with title {title} with icon caution",
        message = applescript_string(&message),
        title = applescript_string(PRODUCT_NAME),
    )
}

fn applescript_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' | '"' => {
                out.push('\\');
                out.push(ch);
            }
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub(crate) fn report_app_launch_error(message: &str) {
    show_app_launch_error(app_launch_dialog_script(message));
}

#[cfg(target_os = "macos")]
fn show_app_launch_error(script: String) {
    let _ = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status();
}

#[cfg(not(target_os = "macos"))]
fn show_app_launch_error(_script: String) {}

fn open_window(page: &str) -> Result<(), Error> {
    let page = page.to_owned();
    tauri::Builder::default()
        .setup(move |app| {
            let url = url::Url::parse(&page).map_err(|err| -> Box<dyn std::error::Error> {
                format!("invalid window address: {err}").into()
            })?;
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(url))
                .title(PRODUCT_NAME)
                .inner_size(1120.0, 800.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .map_err(|err| Error::Io(std::io::Error::other(err.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn window_url_is_the_loopback_page() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 47231);
        assert_eq!(window_url(addr), "http://127.0.0.1:47231");
    }

    #[test]
    fn opening_the_app_bundle_starts_the_window() {
        let exe = "/Applications/Mac Storage Advisor.app/Contents/MacOS/mac-storage";
        let args = vec![exe.to_owned()];
        assert!(launched_as_mac_app(&args, Some(exe)));
        assert!(launched_as_mac_app(&args, None));
        let with_psn = vec![exe.to_owned(), "-psn_0_987654".to_owned()];
        assert!(launched_as_mac_app(&with_psn, Some(exe)));
    }

    #[test]
    fn a_command_inside_the_app_bundle_stays_a_command() {
        let exe = "/Applications/Mac Storage Advisor.app/Contents/MacOS/mac-storage";
        let args = vec![exe.to_owned(), "scan".to_owned(), "/tmp/fixture".to_owned()];
        assert!(!launched_as_mac_app(&args, Some(exe)));
        let installed = ["mac-storage".to_owned()];
        assert!(!launched_as_mac_app(
            &installed,
            Some("/usr/local/bin/mac-storage")
        ));
        assert!(!launched_as_mac_app(&[], None));
        let resources = ["/tmp/notan.app/Contents/Resources/mac-storage".to_owned()];
        assert!(!launched_as_mac_app(&resources, None));
    }

    #[test]
    fn app_launch_dialog_escapes_the_message() {
        let script = app_launch_dialog_script("could not listen on \"127.0.0.1\"\nretry");
        assert!(script.contains("could not listen on \\\"127.0.0.1\\\" retry"));
        assert!(script.contains("with title \"Mac Storage Advisor\""));
        assert!(script.contains("buttons {\"OK\"}"));
        assert!(!script.contains('\n'));
    }

    #[test]
    fn mac_bundle_config_names_this_app_and_its_icons() {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let config: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(manifest.join("tauri.conf.json")).expect("tauri config"),
        )
        .expect("tauri config json");
        assert_eq!(config["productName"], PRODUCT_NAME);
        assert_eq!(config["identifier"], "com.mac-storage.mac-storage-advisor");
        assert!(config.get("version").is_none());
        assert_eq!(config["bundle"]["active"], true);
        assert_eq!(config["bundle"]["category"], "Utility");
        assert_eq!(config["bundle"]["macOS"]["minimumSystemVersion"], "11.0");
        assert_eq!(config["bundle"]["macOS"]["signingIdentity"], "-");
        let targets = config["bundle"]["targets"]
            .as_array()
            .expect("bundle targets");
        assert!(targets.iter().any(|target| target == "app"));
        assert!(targets.iter().any(|target| target == "dmg"));
        for icon in config["bundle"]["icon"].as_array().expect("icons") {
            let name = icon.as_str().expect("icon path");
            let path = manifest.join(name);
            assert!(path.is_file(), "{}", path.display());
            if name.ends_with(".png") {
                let bytes = std::fs::read(&path).expect("png");
                let (width, height, color_type) = png_header(&bytes);
                assert_eq!(width, height);
                assert_eq!(color_type, 6, "icon png keeps an alpha channel");
                match name {
                    "icons/32x32.png" => assert_eq!(width, 32),
                    "icons/128x128.png" => assert_eq!(width, 128),
                    "icons/128x128@2x.png" => assert_eq!(width, 256),
                    "icons/icon.png" => assert_eq!(width, 1024),
                    other => panic!("unexpected png {other}"),
                }
            }
            if name.ends_with(".icns") {
                let bytes = std::fs::read(&path).expect("icns");
                assert!(bytes.starts_with(b"icns"));
                let declared = u32::from_be_bytes(bytes[4..8].try_into().expect("icns size"));
                assert_eq!(declared as usize, bytes.len());
            }
        }
        let plist = std::fs::read_to_string(manifest.join("Info.plist")).expect("Info.plist");
        assert!(plist.contains("NSDownloadsFolderUsageDescription"));
        assert!(plist.contains("NSDesktopFolderUsageDescription"));
        assert!(plist.contains("NSDocumentsFolderUsageDescription"));
        assert!(!plist.contains("app-sandbox"));
    }

    fn png_header(bytes: &[u8]) -> (u32, u32, u8) {
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(&bytes[12..16], b"IHDR");
        let width = u32::from_be_bytes(bytes[16..20].try_into().expect("width"));
        let height = u32::from_be_bytes(bytes[20..24].try_into().expect("height"));
        let color_type = bytes[25];
        (width, height, color_type)
    }
}
