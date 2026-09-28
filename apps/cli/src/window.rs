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

#[derive(Debug, Args)]
pub struct WindowArgs {
    /// Port for the local page. The window loads 127.0.0.1 on this port.
    #[arg(long, default_value_t = 47231)]
    port: u16,

    /// SQLite path. Overrides the MAC_STORAGE_DB environment variable.
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,
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
}
