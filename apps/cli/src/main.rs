//! `mac-storage` command line.

#![forbid(unsafe_code)]

mod query;
mod report;
mod size;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use mac_storage_common::{Error, ScanTarget, PRODUCT_NAME};
use mac_storage_scanner::scan_path;
use mac_storage_storage::{resolve_db_path, Database};

#[derive(Debug, Parser)]
#[command(
    name = "mac-storage",
    version,
    about = PRODUCT_NAME,
    after_long_help = "\
Local, non-destructive storage inventory. `scan` records metadata and does not read file contents.
`duplicates` hashes stored regular files locally with BLAKE3. Hashes stay in the local database.
Nothing is uploaded or deleted. Redundant bytes are not a promise of free disk space.

Examples:
  mac-storage scan ~/Downloads
  mac-storage scan ~/Downloads --json
  mac-storage duplicates
  mac-storage duplicates --verify --json
  mac-storage large-files --limit 20
  mac-storage history

Planned commands (not implemented):
  developer, report, recommendations, doctor

--threads is accepted and stored. Scanning and hashing run on one thread.
Exclusions: a name (node_modules), a path prefix (sub/dir or /abs/path), or a glob
(*.dmg, Downloads/*.dmg, **/*.log). See docs/architecture.md.
Protected macOS prefixes (/System, /private, /bin, /sbin, /usr, /Library) are skipped
only when they sit inside the scan root. Passing one of those paths as the root is refused
unless --allow-protected-roots is set. An explicit root under one of them, such as
/usr/local/myproject, is scanned."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Recursively scan a directory and store metadata in the local database.
    Scan(ScanArgs),
    /// Group identical file contents from a stored scan. Does not delete anything.
    Duplicates(query::DuplicatesArgs),
    /// List the largest regular files from a stored scan.
    LargeFiles(query::LargeArgs),
    /// List scans already stored in the local database.
    History(query::HistoryArgs),
}

#[derive(Debug, Args)]
struct ScanArgs {
    /// Directory to scan. Exclusions do not skip this root itself.
    path: PathBuf,

    /// Print a stable JSON object on stdout. Logs stay on stderr.
    #[arg(long)]
    json: bool,

    /// Debug logging on stderr. File contents are never logged.
    #[arg(long, conflicts_with = "quiet")]
    verbose: bool,

    /// Suppress the human summary and info logs.
    #[arg(long)]
    quiet: bool,

    /// Exclude a path or pattern. Repeatable. See the long help for the rules.
    #[arg(long = "exclude", value_name = "PATH_OR_PATTERN")]
    exclude: Vec<String>,

    /// Requested worker threads. Recorded only; this version is single-threaded.
    #[arg(long, default_value_t = 1)]
    threads: u32,

    /// Count every regular file, but do not persist or list those below this logical size.
    #[arg(long, value_name = "BYTES", default_value = "0", value_parser = size::parse_byte_size)]
    min_size: u64,

    /// Redact paths in diagnostic logs. JSON output and the database still store paths.
    #[arg(long)]
    redact_paths: bool,

    /// Dev override: allow a scan root that is exactly a protected macOS prefix.
    #[arg(long)]
    allow_protected_roots: bool,

    /// SQLite path. Overrides the MAC_STORAGE_DB environment variable.
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<(), Error> {
    match cli.command {
        Command::Scan(args) => scan_command(args),
        Command::Duplicates(args) => query::duplicates_command(args),
        Command::LargeFiles(args) => query::large_files_command(args),
        Command::History(args) => query::history_command(args),
    }
}

fn scan_command(args: ScanArgs) -> Result<(), Error> {
    if args.threads == 0 {
        return Err(Error::Usage(
            "--threads must be at least 1; scanning is single-threaded in this version".into(),
        ));
    }
    init_tracing(args.verbose, args.quiet, args.json);

    let mut target = ScanTarget::new(args.path);
    target.exclusions = args.exclude;
    target.min_logical_size = args.min_size;
    target.threads = args.threads;
    target.allow_protected_roots = args.allow_protected_roots;
    target.redact_paths = args.redact_paths;

    let snapshot = scan_path(&target).map_err(|err| Error::Scan(err.to_string()))?;
    let db_path =
        resolve_db_path(args.db.as_deref()).map_err(|err| Error::Storage(err.to_string()))?;
    let mut database = Database::open(&db_path).map_err(|err| Error::Storage(err.to_string()))?;
    let scan = database
        .save_scan(&snapshot)
        .map_err(|err| Error::Storage(err.to_string()))?;
    let report = report::build_report(&scan, &snapshot, &db_path);

    if args.json {
        report::write_json(&report, std::io::stdout())?;
    } else if !args.quiet {
        print!("{}", report::format_human(&report));
    }
    Ok(())
}

pub(crate) fn init_tracing(verbose: bool, quiet: bool, json: bool) {
    let default_level = if verbose {
        "debug"
    } else if quiet || json {
        "warn"
    } else {
        "info"
    };
    let directive = match std::env::var("RUST_LOG") {
        Ok(value) if !value.is_empty() => value,
        _ => default_level.to_owned(),
    };
    let filter = tracing_subscriber::EnvFilter::try_new(directive)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_level));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}
