//! Where Magi's diagnostics go.
//!
//! Until now: nowhere. `tracing_subscriber::fmt()` writes to stdout, and a tray app opened
//! from Finder has no stdout anybody is reading — macOS captures its own instrumentation of
//! the process into the unified log, but not the application's own output. Checked rather
//! than assumed: `log show --predicate 'process == "magi"'` over a real session returns
//! nineteen thousand lines of XPC, AppKit and ScreenCaptureKit chatter and **zero** lines
//! from any `magi::` target.
//!
//! That is the wrong shape for this app in particular. The documented failure mode is a
//! background process whose hotkey quietly stops working — no window to crash, no terminal
//! to print to. When that happens on someone else's Mac there has to be something to ask
//! them for.
//!
//! ## Why the privacy work came first
//!
//! A log nobody can read is also a log nobody can leak. Giving it a file removes that
//! accident, so what Magi writes had to be audited before it was made durable — and the
//! audit found things: the deictic phrase that triggered a capture (a literal fragment of
//! the user's question), a provider's HTTP rejection body (which can quote the request
//! back), the model's own tool arguments (which summarise what was asked), and absolute
//! paths carrying the account name. Those were removed in the same change that added this
//! file, because the order matters: the file is what turns each of them from a smell into a
//! disclosure.
//!
//! The rule that came out of it, recorded in [`LlmError::log_summary`]: **what Magi wrote
//! travels, what the user or the model wrote does not.** Not sensitivity in the abstract —
//! authorship.
//!
//! [`LlmError::log_summary`]: crate::llm::provider::LlmError::log_summary
//!
//! ## Two sinks, not one
//!
//! stdout stays. `RUST_LOG=magi=debug npm run tauri dev` is how this is read during
//! development and replacing it with a file would make the common case worse to serve the
//! rare one. The file is added beside it, with colour off — ANSI escapes are invisible in a
//! terminal and are line noise in a text file somebody pastes into an issue.

use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

/// How many days of logs to keep.
///
/// A week. Long enough that "it started doing this a few days ago" is answerable, short
/// enough that a background app running for a year does not quietly accumulate a directory
/// nobody looks at. Unbounded growth in a file the user did not ask for is its own bug.
const KEEP_DAYS: usize = 7;

/// `~/Library/Logs/Magi`, if a home directory can be found.
///
/// The Apple-sanctioned location, which is what makes it discoverable — Console.app lists
/// it, and "the Logs folder in your Library" is an instruction that can be given over a bug
/// report without a screenshot.
///
/// Named `Magi` rather than `dev.magi.app`. Tauri's own `app_log_dir` would use the bundle
/// identifier, which is correct and unreadable; a user hunting for this in Finder is looking
/// for the app's name. It also frees this from Tauri's setup order — logging starts before
/// there is an `AppHandle` to ask.
pub fn directory() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Logs/Magi"))
}

/// Start logging to stdout and, if it can, to a rolling file.
///
/// **The returned guard must be held for as long as the process runs.** It flushes the
/// background writer; dropping it early stops the file receiving anything, silently and
/// with no error anywhere — which is the exact failure this module exists to end. `run()`
/// binds it for the lifetime of the Tauri app.
///
/// Returns `None` when there is no file to write to. That is not an error worth refusing to
/// start over: a missing log directory costs a bug report, and refusing to launch costs the
/// application. stdout is configured either way.
#[must_use = "dropping the guard stops the log file receiving anything"]
pub fn init() -> Option<WorkerGuard> {
    let filter =
        || EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("magi=info,warn"));

    let stdout = fmt::layer()
        .with_writer(std::io::stdout)
        .with_filter(filter());

    let Some(file_layer) = file_appender() else {
        tracing_subscriber::registry().with(stdout).init();
        tracing::warn!("no log file: could not open the log directory");
        return None;
    };

    let (writer, guard) = tracing_appender::non_blocking(file_layer.0);
    let file = fmt::layer()
        // No colour. ANSI escapes are invisible in a terminal and are line noise in a file
        // somebody opens in TextEdit or pastes into an issue.
        .with_ansi(false)
        .with_writer(writer)
        .with_filter(filter());

    tracing_subscriber::registry()
        .with(stdout)
        .with(file)
        .init();

    tracing::info!(
        directory = %file_layer.1.display(),
        keep_days = KEEP_DAYS,
        "logging to file"
    );

    Some(guard)
}

/// The rolling appender and the directory it writes to, or `None` if it cannot be built.
fn file_appender() -> Option<(RollingFileAppender, PathBuf)> {
    let directory = directory()?;
    let appender = build_appender(&directory)?;

    Some((appender, directory))
}

/// The appender itself, separated from where it lives so it can be built somewhere else.
///
/// Split out for the test: `directory` reads `HOME`, and a test that writes to the real
/// `~/Library/Logs/Magi` would leave files behind on the machine running it.
fn build_appender(directory: &std::path::Path) -> Option<RollingFileAppender> {
    // Created here rather than left to the appender, so a permissions problem is discovered
    // now and falls back to stdout, instead of surfacing later as an appender that silently
    // writes nothing.
    if let Err(error) = std::fs::create_dir_all(directory) {
        eprintln!("magi: could not create {}: {error}", directory.display());
        return None;
    }

    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("magi")
        .filename_suffix("log")
        .max_log_files(KEEP_DAYS)
        .build(directory)
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directory_is_the_one_macos_expects() {
        // Console.app lists ~/Library/Logs, which is what makes this findable without a
        // screenshot. Anywhere else and the instruction becomes a file path to read out.
        let Some(directory) = directory() else {
            // No HOME in this environment; nothing to assert.
            return;
        };

        assert!(
            directory.ends_with("Library/Logs/Magi"),
            "got {directory:?}"
        );
    }

    #[test]
    fn the_directory_is_named_for_the_app_not_its_identifier() {
        let Some(directory) = directory() else {
            return;
        };

        // `dev.magi.app` is correct and unreadable. Someone looking for this in Finder is
        // looking for the name on the tray icon.
        let name = directory.file_name().unwrap_or_default().to_string_lossy();
        assert_eq!(name, "Magi");
        assert!(!name.contains('.'), "an identifier leaked into the path");
    }

    #[test]
    fn the_appender_creates_its_directory_and_writes_something() {
        // The failure this catches is the quiet one: an appender that builds, accepts
        // writes, and produces no file — which presents as logging having silently
        // stopped, the exact symptom this module exists to remove.
        use std::io::Write;

        let directory =
            std::env::temp_dir().join(format!("magi-log-test-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&directory);

        let mut appender = build_appender(&directory).expect("the appender should build");
        writeln!(appender, "a line").expect("the appender should accept a write");
        appender.flush().expect("the appender should flush");

        let written: Vec<_> = std::fs::read_dir(&directory)
            .expect("the directory should have been created")
            .filter_map(Result::ok)
            .collect();

        assert_eq!(written.len(), 1, "expected exactly one log file");
        let name = written[0].file_name().to_string_lossy().to_string();
        assert!(name.starts_with("magi."), "unexpected name: {name}");
        assert!(name.ends_with(".log"), "unexpected name: {name}");

        let contents = std::fs::read_to_string(written[0].path()).expect("readable");
        assert!(contents.contains("a line"), "nothing was written");

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_directory_that_cannot_be_created_is_survived() {
        // A file where the directory should be. The app must still start — a missing log
        // costs a bug report, refusing to launch costs the application.
        let path = std::env::temp_dir().join(format!("magi-log-blocked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::write(&path, b"not a directory").expect("could write the blocker");

        assert!(build_appender(&path).is_none());

        let _ = std::fs::remove_file(&path);
    }
}
