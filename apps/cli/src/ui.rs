//! Local page for the same scan, duplicate, and suggestion commands.
//!
//! The listener is `127.0.0.1` only. Requests with another Host header are
//! refused. The page is not a shell and does not upload or delete files.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use clap::Args;
use mac_storage_common::{Error, ScanReport, ScanTarget, PRODUCT_NAME, PRODUCT_VERSION};
use mac_storage_scanner::{scan_path, scan_path_reporting};
use mac_storage_storage::{resolve_db_path, Database};
use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::{advise, query, report, size};

const PAGE: &str = include_str!("ui_page.html");
const MAX_BODY: usize = 256 * 1024;
pub const DEFAULT_PORT: u16 = 47231;

#[derive(Debug, Args)]
pub struct UiArgs {
    /// Port on 127.0.0.1. Other addresses are refused.
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,

    /// SQLite path. Overrides the MAC_STORAGE_DB environment variable.
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,
}

struct App {
    database: PathBuf,
    listen: SocketAddr,
    scan: Arc<Mutex<ScanJob>>,
}

struct ScanJob {
    phase: &'static str,
    root: String,
    directories: u64,
    files: u64,
    logical_bytes: u64,
    errors: u64,
    current_dir: String,
    message: String,
    report: Option<ScanReport>,
}

impl ScanJob {
    fn idle() -> Self {
        Self {
            phase: "idle",
            root: String::new(),
            directories: 0,
            files: 0,
            logical_bytes: 0,
            errors: 0,
            current_dir: String::new(),
            message: String::new(),
            report: None,
        }
    }
}

#[derive(Serialize)]
struct ProgressBody<'a> {
    phase: &'a str,
    root: &'a str,
    directories: u64,
    files: u64,
    logical_bytes: u64,
    errors: u64,
    current_dir: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    report: Option<&'a ScanReport>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
}

#[derive(Serialize)]
struct StatusBody<'a> {
    product: &'static str,
    version: &'static str,
    database: &'a Path,
    listen: String,
    mode: &'static str,
    home: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct ScanBody {
    path: String,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    min_size: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DuplicatesBody {
    #[serde(default)]
    scan: Option<i64>,
    #[serde(default)]
    verify: bool,
}

#[derive(Debug, Deserialize)]
struct CommandBody {
    line: String,
}

#[derive(Debug, Deserialize)]
struct TrashBody {
    #[serde(default)]
    scan: Option<i64>,
    paths: Vec<PathBuf>,
    #[serde(default)]
    confirmation: Option<String>,
}

#[derive(Serialize)]
struct CommandResult {
    text: String,
    scan_id: Option<i64>,
}

enum PageCommand {
    Help,
    Scan {
        path: PathBuf,
        exclude: Vec<String>,
        min_size: u64,
    },
    Duplicates {
        scan: Option<i64>,
        verify: bool,
    },
    LargeFiles {
        scan: Option<i64>,
        limit: u64,
        min_size: u64,
    },
    Folders {
        scan: Option<i64>,
        depth: u32,
    },
    History {
        limit: u64,
    },
    Review {
        scan: Option<i64>,
        older_than: u64,
        recommendations_only: bool,
    },
    Trends {
        limit: u64,
    },
    Trash {
        scan: Option<i64>,
        paths: Vec<PathBuf>,
        confirm: Option<String>,
    },
    Doctor,
}

struct ParsedCommand {
    json: bool,
    command: PageCommand,
}

pub fn ui_command(args: UiArgs) -> Result<(), Error> {
    if args.port == 0 {
        return Err(Error::Usage("--port must be between 1 and 65535".into()));
    }
    let database =
        resolve_db_path(args.db.as_deref()).map_err(|err| Error::Storage(err.to_string()))?;
    Database::open(&database).map_err(|err| Error::Storage(err.to_string()))?;
    let (sender, receiver) = std::sync::mpsc::channel();
    let result = serve(
        database,
        args.port,
        sender,
        Arc::new(AtomicBool::new(false)),
        true,
    );
    drop(receiver);
    result
}

pub(crate) fn serve(
    database: PathBuf,
    port: u16,
    bound: Sender<SocketAddr>,
    stop: Arc<AtomicBool>,
    announce: bool,
) -> Result<(), Error> {
    let listen_at = format!("127.0.0.1:{port}");
    let server = Server::http(&listen_at).map_err(|err| {
        Error::Io(std::io::Error::other(format!(
            "could not listen on {listen_at}: {err}"
        )))
    })?;
    let listen = server.server_addr().to_ip().ok_or_else(|| {
        Error::Io(std::io::Error::other(
            "the UI listener did not bind a TCP address",
        ))
    })?;
    if announce {
        println!(
            "{PRODUCT_NAME} {PRODUCT_VERSION}\n\
             Interactive UI: http://{listen}\n\
             Command line: mac-storage scan, duplicates, large-files, history, analyze, recommendations, trends\n\
             Database: {}\n\
             Listening on this computer only. Press Ctrl-C to stop.\n\
             Nothing is uploaded or deleted.",
            database.display()
        );
    }
    let _ = bound.send(listen);
    let app = App {
        database,
        listen,
        scan: Arc::new(Mutex::new(ScanJob::idle())),
    };
    while !stop.load(Ordering::Relaxed) {
        match server.recv_timeout(Duration::from_millis(200)) {
            Ok(Some(request)) => respond(request, &app),
            Ok(None) => {}
            Err(err) => return Err(Error::Io(err)),
        }
    }
    Ok(())
}

fn respond(mut request: Request, app: &App) {
    let method = match request.method() {
        Method::Get => "GET",
        Method::Post => "POST",
        _ => {
            let _ = send(
                request,
                405,
                "application/json; charset=utf-8",
                &json_error("only GET and POST are accepted"),
            );
            return;
        }
    };
    let url = request.url().to_owned();
    let host = host_header(&request).map(str::to_owned);
    let body = match read_body(&mut request) {
        Ok(body) => body,
        Err(err) => {
            let _ = send(
                request,
                413,
                "application/json; charset=utf-8",
                &json_error(&err.to_string()),
            );
            return;
        }
    };
    let reply = dispatch(app, method, &url, host.as_deref(), &body);
    let _ = send(request, reply.status, reply.content_type, &reply.body);
}

struct Reply {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

fn dispatch(app: &App, method: &str, url: &str, host: Option<&str>, body: &[u8]) -> Reply {
    let Some(host) = host else {
        return json_reply(400, &json_error("Host header is required"));
    };
    if !host_allowed(host, app.listen.port()) {
        return json_reply(403, &json_error("this page accepts only 127.0.0.1"));
    }
    let (path, query) = split_url(url);
    let result = match (method, path) {
        ("GET", "/") => {
            return Reply {
                status: 200,
                content_type: "text/html; charset=utf-8",
                body: PAGE.as_bytes().to_vec(),
            };
        }
        ("GET", "/api/status") => status(app),
        ("GET", "/api/scan/progress") => scan_progress(app),
        ("GET", "/api/history") => history(app, &query),
        ("GET", "/api/large-files") => large_files(app, &query),
        ("GET", "/api/folders") => folders(app, &query),
        ("GET", "/api/analyze") => review(app, &query, false),
        ("GET", "/api/recommendations") => review(app, &query, true),
        ("GET", "/api/trends") => trends(app, &query),
        ("POST", "/api/scan") => scan(app, body),
        ("POST", "/api/scan/start") => start_scan(app, body),
        ("POST", "/api/duplicates") => duplicates(app, body),
        ("GET", "/api/doctor") => doctor(app),
        ("POST", "/api/trash") => trash(app, body),
        ("POST", "/api/command") => command(app, body),
        _ => Err(Error::Usage("unknown request".into())),
    };
    match result {
        Ok(bytes) => Reply {
            status: 200,
            content_type: "application/json; charset=utf-8",
            body: bytes,
        },
        Err(err) => {
            let status = if matches!(err, Error::Io(_)) {
                500
            } else {
                400
            };
            json_reply(status, &json_error(&err.to_string()))
        }
    }
}

fn status(app: &App) -> Result<Vec<u8>, Error> {
    to_json(&StatusBody {
        product: PRODUCT_NAME,
        version: PRODUCT_VERSION,
        database: &app.database,
        listen: app.listen.to_string(),
        mode: "ui",
        home: home_directory(),
    })
}

fn home_directory() -> Option<PathBuf> {
    let value = std::env::var_os("HOME")?;
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

fn lock_scan(scan: &Mutex<ScanJob>) -> std::sync::MutexGuard<'_, ScanJob> {
    scan.lock().unwrap_or_else(|err| err.into_inner())
}

fn progress_body(job: &ScanJob) -> ProgressBody<'_> {
    ProgressBody {
        phase: job.phase,
        root: &job.root,
        directories: job.directories,
        files: job.files,
        logical_bytes: job.logical_bytes,
        errors: job.errors,
        current_dir: &job.current_dir,
        message: &job.message,
        report: job.report.as_ref(),
    }
}

fn scan_progress(app: &App) -> Result<Vec<u8>, Error> {
    let job = lock_scan(&app.scan);
    to_json(&progress_body(&job))
}

fn start_scan(app: &App, body: &[u8]) -> Result<Vec<u8>, Error> {
    let request: ScanBody = parse_json(body)?;
    let path = expand_home(request.path.trim());
    if path.as_os_str().is_empty() {
        return Err(Error::Usage("path is required".into()));
    }
    let min_size = size::parse_byte_size(request.min_size.as_deref().unwrap_or("0"))
        .map_err(|err| Error::Usage(err.to_string()))?;
    let exclude = request
        .exclude
        .into_iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    {
        let mut job = lock_scan(&app.scan);
        if job.phase == "walking" || job.phase == "saving" {
            return to_json(&progress_body(&job));
        }
        *job = ScanJob {
            phase: "walking",
            root: path.display().to_string(),
            directories: 0,
            files: 0,
            logical_bytes: 0,
            errors: 0,
            current_dir: path.display().to_string(),
            message: String::new(),
            report: None,
        };
    }
    let scan = Arc::clone(&app.scan);
    let database = app.database.clone();
    thread::spawn(move || {
        let mut target = ScanTarget::new(path);
        target.exclusions = exclude;
        target.min_logical_size = min_size;
        target.threads = 1;
        let mut last_report = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(Instant::now);
        let outcome = scan_path_reporting(&target, |progress| {
            if last_report.elapsed() < Duration::from_millis(200) {
                return;
            }
            last_report = Instant::now();
            let mut job = lock_scan(&scan);
            if job.phase != "walking" {
                return;
            }
            job.directories = progress.directories;
            job.files = progress.files;
            job.logical_bytes = progress.logical_bytes;
            job.errors = progress.errors;
            job.current_dir.clone_from(&progress.current_dir);
        });
        match outcome {
            Ok(snapshot) => {
                {
                    let mut job = lock_scan(&scan);
                    job.phase = "saving";
                    job.directories = snapshot.statistics.directories_scanned;
                    job.files = snapshot.statistics.files_scanned;
                    job.logical_bytes = snapshot.statistics.logical_bytes;
                    job.errors = snapshot.statistics.errors;
                }
                match save_snapshot(&database, &snapshot) {
                    Ok(report) => {
                        let mut job = lock_scan(&scan);
                        job.phase = "done";
                        job.message.clear();
                        job.report = Some(report);
                    }
                    Err(err) => {
                        let mut job = lock_scan(&scan);
                        job.phase = "error";
                        job.message = err.to_string();
                    }
                }
            }
            Err(err) => {
                let mut job = lock_scan(&scan);
                job.phase = "error";
                job.message = err.to_string();
            }
        }
    });
    let job = lock_scan(&app.scan);
    to_json(&progress_body(&job))
}

fn save_snapshot(
    database: &Path,
    snapshot: &mac_storage_common::ScanSnapshot,
) -> Result<ScanReport, Error> {
    let mut db = Database::open(database).map_err(|err| Error::Storage(err.to_string()))?;
    let scan = db
        .save_scan(snapshot)
        .map_err(|err| Error::Storage(err.to_string()))?;
    Ok(report::build_report(&scan, snapshot, database))
}

fn history(app: &App, query: &HashMap<String, String>) -> Result<Vec<u8>, Error> {
    let limit = query_u64(query, "limit", 20)?;
    to_json(&query::run_history(Some(&app.database), limit)?)
}

fn folders(app: &App, query: &HashMap<String, String>) -> Result<Vec<u8>, Error> {
    let depth = match query.get("depth").map(String::as_str) {
        Some(value) if !value.trim().is_empty() => value
            .parse::<u32>()
            .map_err(|_| Error::Usage("--depth must be an integer".into()))?,
        _ => 1,
    };
    to_json(&crate::folders::run_folders(
        Some(app.database.as_path()),
        query_i64(query, "scan")?,
        depth,
    )?)
}

fn large_files(app: &App, query: &HashMap<String, String>) -> Result<Vec<u8>, Error> {
    let limit = query_u64(query, "limit", 20)?;
    let min_size = query_size(query, "min_size")?;
    to_json(&query::run_large_files(
        Some(&app.database),
        query_i64(query, "scan")?,
        limit,
        min_size,
    )?)
}

fn review(
    app: &App,
    query: &HashMap<String, String>,
    recommendations_only: bool,
) -> Result<Vec<u8>, Error> {
    let older = match query.get("older_than").map(String::as_str) {
        Some(value) if !value.trim().is_empty() => {
            size::parse_days(value).map_err(|err| Error::Usage(err.to_string()))?
        }
        _ => 180,
    };
    let report = advise::run_review(Some(&app.database), query_i64(query, "scan")?, older)?;
    if recommendations_only {
        to_json(&advise::recommendations_view(&report))
    } else {
        to_json(&report)
    }
}

fn trends(app: &App, query: &HashMap<String, String>) -> Result<Vec<u8>, Error> {
    to_json(&advise::run_trends(
        Some(&app.database),
        query_u64(query, "limit", 20)?,
    )?)
}

fn scan(app: &App, body: &[u8]) -> Result<Vec<u8>, Error> {
    let request: ScanBody = parse_json(body)?;
    let path = expand_home(request.path.trim());
    if path.as_os_str().is_empty() {
        return Err(Error::Usage("path is required".into()));
    }
    let min_size = size::parse_byte_size(request.min_size.as_deref().unwrap_or("0"))
        .map_err(|err| Error::Usage(err.to_string()))?;
    let exclude = request
        .exclude
        .into_iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect();
    to_json(&run_scan(&app.database, path, exclude, min_size)?)
}

fn duplicates(app: &App, body: &[u8]) -> Result<Vec<u8>, Error> {
    let request: DuplicatesBody = if body.is_empty() {
        DuplicatesBody {
            scan: None,
            verify: false,
        }
    } else {
        parse_json(body)?
    };
    to_json(&query::run_duplicates(
        Some(&app.database),
        request.scan,
        request.verify,
    )?)
}

fn doctor(app: &App) -> Result<Vec<u8>, Error> {
    to_json(&crate::doctor::run_doctor(Some(app.database.as_path()))?)
}

fn trash(app: &App, body: &[u8]) -> Result<Vec<u8>, Error> {
    let request: TrashBody = parse_json(body)?;
    let confirmation = request
        .confirmation
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    to_json(&crate::trash_cmd::run_trash(
        Some(app.database.as_path()),
        request.scan,
        &request.paths,
        confirmation,
    )?)
}

fn command(app: &App, body: &[u8]) -> Result<Vec<u8>, Error> {
    let request: CommandBody = parse_json(body)?;
    let parsed = parse_line(&request.line)?;
    let outcome = execute(app, parsed)?;
    to_json(&outcome)
}

fn execute(app: &App, parsed: ParsedCommand) -> Result<CommandResult, Error> {
    let db = Some(app.database.as_path());
    match parsed.command {
        PageCommand::Help => Ok(CommandResult {
            text: help_text().to_owned(),
            scan_id: None,
        }),
        PageCommand::Scan {
            path,
            exclude,
            min_size,
        } => {
            let report = run_scan(&app.database, path, exclude, min_size)?;
            let scan_id = Some(report.scan_id);
            let text = if parsed.json {
                pretty(&report)?
            } else {
                report::format_human(&report)
            };
            Ok(CommandResult { text, scan_id })
        }
        PageCommand::Duplicates { scan, verify } => {
            let report = query::run_duplicates(db, scan, verify)?;
            finish(parsed.json, &report, query::format_duplicates(&report))
        }
        PageCommand::LargeFiles {
            scan,
            limit,
            min_size,
        } => {
            let report = query::run_large_files(db, scan, limit, min_size)?;
            finish(parsed.json, &report, query::format_large(&report))
        }
        PageCommand::Folders { scan, depth } => {
            let report = crate::folders::run_folders(db, scan, depth)?;
            finish(
                parsed.json,
                &report,
                crate::folders::format_folders(&report),
            )
        }
        PageCommand::History { limit } => {
            let report = query::run_history(db, limit)?;
            finish(parsed.json, &report, query::format_history(&report))
        }
        PageCommand::Review {
            scan,
            older_than,
            recommendations_only,
        } => {
            let report = advise::run_review(db, scan, older_than)?;
            let text = if parsed.json {
                if recommendations_only {
                    pretty(&advise::recommendations_view(&report))?
                } else {
                    pretty(&report)?
                }
            } else if recommendations_only {
                advise::format_recommendations(&report)
            } else {
                advise::format_analyze(&report)
            };
            let scan_id = scan_id_of(&report);
            Ok(CommandResult { text, scan_id })
        }
        PageCommand::Trends { limit } => {
            let report = advise::run_trends(db, limit)?;
            finish(parsed.json, &report, advise::format_trends(&report))
        }
        PageCommand::Trash {
            scan,
            paths,
            confirm,
        } => {
            let report = crate::trash_cmd::run_trash(db, scan, &paths, confirm.as_deref())?;
            let scan_id = Some(report.scan_id);
            let text = if parsed.json {
                pretty(&report)?
            } else {
                crate::trash_cmd::format_trash(&report)
            };
            Ok(CommandResult { text, scan_id })
        }
        PageCommand::Doctor => {
            let report = crate::doctor::run_doctor(db)?;
            let scan_id = report.newest_scan.as_ref().map(|scan| scan.id);
            let text = if parsed.json {
                pretty(&report)?
            } else {
                crate::doctor::format_doctor(&report)
            };
            Ok(CommandResult { text, scan_id })
        }
    }
}

fn finish<T: Serialize>(json: bool, report: &T, human: String) -> Result<CommandResult, Error> {
    let text = if json { pretty(report)? } else { human };
    Ok(CommandResult {
        text,
        scan_id: scan_id_of(report),
    })
}

fn run_scan(
    database: &Path,
    path: PathBuf,
    exclude: Vec<String>,
    min_size: u64,
) -> Result<ScanReport, Error> {
    let mut target = ScanTarget::new(path);
    target.exclusions = exclude;
    target.min_logical_size = min_size;
    target.threads = 1;
    let snapshot = scan_path(&target).map_err(|err| Error::Scan(err.to_string()))?;
    save_snapshot(database, &snapshot)
}

fn help_text() -> &'static str {
    "\
Mac Storage Advisor page commands (not a system shell):
  help
  scan <PATH> [--exclude NAME] [--min-size SIZE] [--json]
  duplicates [--scan ID] [--verify] [--json]
  large-files [--scan ID] [--limit N] [--min-size SIZE] [--json]
  folders [--scan ID] [--depth N] [--json]
  history [--limit N] [--json]
  analyze [--scan ID] [--older-than DAYS] [--json]
  recommendations [--scan ID] [--older-than DAYS] [--json]
  trends [--limit N] [--json]
  trash --path PATH [--path PATH] [--scan ID] [--confirm PHRASE] [--json]
  doctor [--json]

A leading ~/ is your home directory. Pipes, semicolons, and other programs are refused.
trash moves a path only when --confirm is exactly: move to trash. Otherwise nothing is moved.
Nothing is uploaded or permanently deleted.
"
}

fn parse_line(line: &str) -> Result<ParsedCommand, Error> {
    if line.contains([';', '|', '&', '`']) {
        return Err(Error::Usage(
            "this is not a shell; pipes and extra commands are refused".into(),
        ));
    }
    let words = split_words(line).map_err(Error::Usage)?;
    if words.is_empty() {
        return Err(Error::Usage("type help to see the commands".into()));
    }
    let verb = words[0].as_str();
    let mut json = false;
    let mut exclude = Vec::new();
    let mut min_size = 0u64;
    let mut scan = None;
    let mut verify = false;
    let mut limit = 20u64;
    let mut older_than = 180u64;
    let mut depth = 1u32;
    let mut trash_paths = Vec::new();
    let mut confirm: Option<String> = None;
    let mut positionals = Vec::new();
    let mut index = 1;
    while index < words.len() {
        let word = words[index].as_str();
        let (name, inline) = match word.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (word, None),
        };
        let take_value = |index: &mut usize| -> Result<String, Error> {
            if let Some(value) = inline.clone() {
                *index += 1;
                return Ok(value);
            }
            *index += 1;
            let value = words
                .get(*index)
                .cloned()
                .ok_or_else(|| Error::Usage(format!("{name} needs a value")))?;
            *index += 1;
            Ok(value)
        };
        match name {
            "--json" => {
                json = true;
                index += 1;
            }
            "--verify" => {
                verify = true;
                index += 1;
            }
            "--exclude" => exclude.push(take_value(&mut index)?),
            "--min-size" => {
                let raw = take_value(&mut index)?;
                min_size =
                    size::parse_byte_size(&raw).map_err(|err| Error::Usage(err.to_string()))?;
            }
            "--scan" => {
                let raw = take_value(&mut index)?;
                scan = Some(parse_i64(&raw, "--scan")?);
            }
            "--depth" => {
                let raw = take_value(&mut index)?;
                depth = raw
                    .parse::<u32>()
                    .map_err(|_| Error::Usage("--depth must be an integer".into()))?;
            }
            "--limit" => {
                let raw = take_value(&mut index)?;
                limit = raw
                    .parse::<u64>()
                    .map_err(|_| Error::Usage("--limit must be an integer".into()))?;
            }
            "--older-than" => {
                let raw = take_value(&mut index)?;
                older_than = size::parse_days(&raw).map_err(|err| Error::Usage(err.to_string()))?;
            }
            "--path" => trash_paths.push(PathBuf::from(take_value(&mut index)?)),
            "--confirm" => confirm = Some(take_value(&mut index)?),
            "--db"
            | "--threads"
            | "--verbose"
            | "--quiet"
            | "--allow-protected-roots"
            | "--redact-paths" => {
                return Err(Error::Usage(format!(
                    "{name} is set by the running UI, not by this box"
                )));
            }
            other if other.starts_with('-') => {
                return Err(Error::Usage(format!("unknown flag {other}")));
            }
            other => {
                positionals.push(other.to_owned());
                index += 1;
            }
        }
    }
    if verb != "trash" && (!trash_paths.is_empty() || confirm.is_some()) {
        return Err(Error::Usage(
            "--path and --confirm belong to the trash command".into(),
        ));
    }
    let command = match verb {
        "help" => {
            if !positionals.is_empty() {
                return Err(Error::Usage("help takes no arguments".into()));
            }
            PageCommand::Help
        }
        "scan" => {
            if positionals.len() != 1 {
                return Err(Error::Usage("scan needs one folder path".into()));
            }
            PageCommand::Scan {
                path: expand_home(positionals[0].trim()),
                exclude,
                min_size,
            }
        }
        "duplicates" => {
            reject_positionals(verb, &positionals)?;
            PageCommand::Duplicates { scan, verify }
        }
        "large-files" => {
            reject_positionals(verb, &positionals)?;
            PageCommand::LargeFiles {
                scan,
                limit,
                min_size,
            }
        }
        "folders" => {
            reject_positionals(verb, &positionals)?;
            PageCommand::Folders { scan, depth }
        }
        "history" => {
            reject_positionals(verb, &positionals)?;
            PageCommand::History { limit }
        }
        "analyze" => {
            reject_positionals(verb, &positionals)?;
            PageCommand::Review {
                scan,
                older_than,
                recommendations_only: false,
            }
        }
        "recommendations" => {
            reject_positionals(verb, &positionals)?;
            PageCommand::Review {
                scan,
                older_than,
                recommendations_only: true,
            }
        }
        "trends" => {
            reject_positionals(verb, &positionals)?;
            PageCommand::Trends { limit }
        }
        "trash" => {
            reject_positionals(verb, &positionals)?;
            PageCommand::Trash {
                scan,
                paths: trash_paths,
                confirm,
            }
        }
        "doctor" => {
            reject_positionals(verb, &positionals)?;
            PageCommand::Doctor
        }
        "rm" | "sh" | "bash" | "zsh" | "sudo" => {
            return Err(Error::Usage(format!(
                "{verb} is not available; this page only runs advisor commands"
            )));
        }
        other => {
            return Err(Error::Usage(format!("unknown command {other}; type help")));
        }
    };
    Ok(ParsedCommand { json, command })
}

fn reject_positionals(verb: &str, positionals: &[String]) -> Result<(), Error> {
    if positionals.is_empty() {
        Ok(())
    } else {
        Err(Error::Usage(format!(
            "{verb} does not take a folder; use scan, or --scan ID"
        )))
    }
}

fn split_words(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in line.chars() {
        match ch {
            '"' => quoted = !quoted,
            ch if ch.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            ch => current.push(ch),
        }
    }
    if quoted {
        return Err("unclosed quote".into());
    }
    if !current.is_empty() {
        words.push(current);
    }
    Ok(words)
}

fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = home_dir() {
            return home;
        }
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

fn host_allowed(host: &str, port: u16) -> bool {
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

fn host_header(request: &Request) -> Option<&str> {
    request.headers().iter().find_map(|header| {
        if header.field.equiv("Host") {
            Some(header.value.as_str())
        } else {
            None
        }
    })
}

fn read_body(request: &mut Request) -> Result<Vec<u8>, Error> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let reader = request.as_reader();
    loop {
        let read = reader.read(&mut chunk).map_err(Error::Io)?;
        if read == 0 {
            break;
        }
        if buf.len() + read > MAX_BODY {
            return Err(Error::Usage("request body is too large".into()));
        }
        buf.extend_from_slice(&chunk[..read]);
    }
    Ok(buf)
}

fn split_url(url: &str) -> (&str, HashMap<String, String>) {
    let (path, raw_query) = url.split_once('?').unwrap_or((url, ""));
    let mut query = HashMap::new();
    if !raw_query.is_empty() {
        for pair in raw_query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            query.insert(percent_decode(key), percent_decode(value));
        }
    }
    (path, query)
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    index += 3;
                } else {
                    out.push(b'%');
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_u64(query: &HashMap<String, String>, key: &str, default: u64) -> Result<u64, Error> {
    match query
        .get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        None => Ok(default),
        Some(value) => value
            .parse::<u64>()
            .map_err(|_| Error::Usage(format!("{key} must be an integer"))),
    }
}

fn query_i64(query: &HashMap<String, String>, key: &str) -> Result<Option<i64>, Error> {
    match query
        .get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        None => Ok(None),
        Some(value) => Ok(Some(parse_i64(value, key)?)),
    }
}

fn query_size(query: &HashMap<String, String>, key: &str) -> Result<u64, Error> {
    match query
        .get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        None => Ok(0),
        Some(value) => size::parse_byte_size(value).map_err(|err| Error::Usage(err.to_string())),
    }
}

fn parse_i64(value: &str, name: &str) -> Result<i64, Error> {
    value
        .parse::<i64>()
        .map_err(|_| Error::Usage(format!("{name} must be an integer")))
}

fn parse_json<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(body).map_err(|err| Error::Usage(format!("invalid JSON: {err}")))
}

fn to_json<T: Serialize>(value: &T) -> Result<Vec<u8>, Error> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|err| Error::Scan(format!("failed to write JSON report: {err}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn pretty<T: Serialize>(value: &T) -> Result<String, Error> {
    let mut text = serde_json::to_string_pretty(value)
        .map_err(|err| Error::Scan(format!("failed to write JSON report: {err}")))?;
    text.push('\n');
    Ok(text)
}

fn scan_id_of<T: Serialize>(value: &T) -> Option<i64> {
    serde_json::to_value(value)
        .ok()
        .and_then(|json| json.get("scan_id").and_then(serde_json::Value::as_i64))
}

fn json_error(message: &str) -> Vec<u8> {
    serde_json::to_vec(&ErrorBody { error: message })
        .unwrap_or_else(|_| br#"{"error":"request failed"}"#.to_vec())
}

fn json_reply(status: u16, body: &[u8]) -> Reply {
    let mut owned = body.to_vec();
    if !owned.ends_with(b"\n") {
        owned.push(b'\n');
    }
    Reply {
        status,
        content_type: "application/json; charset=utf-8",
        body: owned,
    }
}

fn send(
    request: Request,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), std::io::Error> {
    let mut response = Response::from_data(body.to_vec()).with_status_code(StatusCode(status));
    for (name, value) in [
        ("Content-Type", content_type),
        ("Cache-Control", "no-store"),
        ("X-Content-Type-Options", "nosniff"),
        (
            "Content-Security-Policy",
            "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; img-src 'none'; base-uri 'none'; form-action 'self'",
        ),
    ] {
        if let Ok(header) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response = response.with_header(header);
        }
    }
    request.respond(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::mpsc;
    use std::thread;

    #[test]
    fn command_parser_accepts_scan_flags_and_refuses_a_shell() {
        let scan =
            parse_line(r#"scan "/tmp/my files" --exclude node_modules --min-size 1KiB"#).unwrap();
        match scan.command {
            PageCommand::Scan {
                path,
                exclude,
                min_size,
            } => {
                assert_eq!(path, PathBuf::from("/tmp/my files"));
                assert_eq!(exclude, vec!["node_modules".to_owned()]);
                assert_eq!(min_size, 1024);
            }
            _ => panic!("expected scan"),
        }
        assert!(parse_line("rm -rf /").is_err());
        assert!(parse_line("scan /tmp | sh").is_err());
        assert!(parse_line("duplicates --db /tmp/other.sqlite").is_err());
        assert!(matches!(
            parse_line("help").unwrap().command,
            PageCommand::Help
        ));
    }

    #[test]
    fn host_check_is_loopback_only() {
        assert!(host_allowed("127.0.0.1:47231", 47231));
        assert!(host_allowed("localhost:47231", 47231));
        assert!(!host_allowed("evil.example:47231", 47231));
        assert!(!host_allowed("127.0.0.1:80", 47231));
        assert!(!host_allowed("0.0.0.0:47231", 47231));
    }

    #[test]
    fn page_and_scan_round_trip_on_localhost() {
        let root = std::env::temp_dir().join(format!(
            "mac-storage-ui-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), b"same-bytes").unwrap();
        std::fs::write(root.join("b.txt"), b"same-bytes").unwrap();
        std::fs::write(root.join("c.txt"), b"other").unwrap();
        let db = root.with_extension("sqlite");
        let _ = std::fs::remove_file(&db);
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let stop_thread = Arc::clone(&stop);
        let database = db.clone();
        let thread = thread::spawn(move || serve(database, 0, sender, stop_thread, false));
        let addr = receiver.recv_timeout(Duration::from_secs(5)).unwrap();

        let page = http(addr, "GET", "/", "");
        assert_eq!(page.0, 200);
        assert!(page.1.contains("Interactive"));
        assert!(page.1.contains("Command line"));
        assert!(page.1.contains("By folder"));
        assert!(page.1.contains("Largest extra copies"));
        assert!(page.1.contains("data-section=\"duplicates\""));
        assert!(page.1.contains("Scan this Mac"));
        assert!(page.1.contains("scan-progress"));
        assert!(!page.1.contains("<script src="));

        let denied = http_host(addr, "GET", "/api/status", "evil.example", "");
        assert_eq!(denied.0, 403);

        let status = http(addr, "GET", "/api/status", "");
        assert_eq!(status.0, 200);
        assert!(status.1.contains(PRODUCT_NAME));
        assert!(status.1.contains(&db.display().to_string()));
        let status_json: serde_json::Value = serde_json::from_str(&status.1).unwrap();
        assert!(status_json.get("home").is_some());

        let scan_body = format!(
            r#"{{"path":"{}","exclude":[],"min_size":"0"}}"#,
            root.display()
                .to_string()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        );
        let started = http(addr, "POST", "/api/scan/start", &scan_body);
        assert_eq!(started.0, 200, "{}", started.1);
        let mut progress_json = serde_json::Value::Null;
        for _ in 0..50 {
            let progress = http(addr, "GET", "/api/scan/progress", "");
            assert_eq!(progress.0, 200, "{}", progress.1);
            progress_json = serde_json::from_str(&progress.1).unwrap();
            let phase = progress_json["phase"].as_str().unwrap_or("");
            if phase == "done" || phase == "error" {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(progress_json["phase"], "done", "{progress_json}");
        assert_eq!(progress_json["report"]["files_scanned"], 3);
        assert!(progress_json["report"].get("contents").is_none());

        let scanned = http(addr, "POST", "/api/scan", &scan_body);
        assert_eq!(scanned.0, 200, "{}", scanned.1);
        let scanned_json: serde_json::Value = serde_json::from_str(&scanned.1).unwrap();
        assert_eq!(scanned_json["files_scanned"], 3);
        assert!(scanned_json.get("contents").is_none());
        let scan_id = scanned_json["scan_id"].as_i64().unwrap();

        let duplicates = http(
            addr,
            "POST",
            "/api/duplicates",
            &format!(r#"{{"scan":{scan_id},"verify":false}}"#),
        );
        assert_eq!(duplicates.0, 200, "{}", duplicates.1);
        let duplicates_json: serde_json::Value = serde_json::from_str(&duplicates.1).unwrap();
        assert_eq!(
            duplicates_json["duplicate_groups"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(duplicates_json.get("file_contents").is_none());

        let large = http(
            addr,
            "GET",
            &format!("/api/large-files?scan={scan_id}&limit=10"),
            "",
        );
        assert_eq!(large.0, 200, "{}", large.1);
        assert!(large.1.contains("a.txt") || large.1.contains("b.txt"));

        let advice = http(
            addr,
            "GET",
            &format!("/api/recommendations?scan={scan_id}"),
            "",
        );
        assert_eq!(advice.0, 200, "{}", advice.1);
        assert!(advice.1.contains("recommendations"));

        let command = http(
            addr,
            "POST",
            "/api/command",
            r#"{"line":"history --limit 5"}"#,
        );
        assert_eq!(command.0, 200, "{}", command.1);
        assert!(command.1.contains("Scans:"));

        let refused = http(addr, "POST", "/api/command", r#"{"line":"rm -rf /tmp"}"#);
        assert_eq!(refused.0, 400);

        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    fn unique_suffix() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
    }

    fn http(addr: SocketAddr, method: &str, path: &str, body: &str) -> (u16, String) {
        http_host(addr, method, path, "127.0.0.1", body)
    }

    fn http_host(
        addr: SocketAddr,
        method: &str,
        path: &str,
        host: &str,
        body: &str,
    ) -> (u16, String) {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        let host_header = if host.contains(':') {
            host.to_owned()
        } else {
            format!("{host}:{}", addr.port())
        };
        let request = format!(
            "{method} {path} HTTP/1.0\r\nHost: {host_header}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        let (head, payload) = raw.split_once("\r\n\r\n").unwrap_or(("", &raw));
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        (status, payload.to_owned())
    }
}
