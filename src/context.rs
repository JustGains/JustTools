//! `just context` — the Windows File Explorer context menu.
//!
//! `install`, `uninstall`, and `status` manage the registration. `run` is what
//! Explorer starts when an entry is chosen: it turns the entry's ID and the
//! selected paths into the same headless command a terminal user would type,
//! shows that command, runs it, and keeps the window readable afterwards.

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::style::Stylize;
use justtools_menu::{self as menu, Action, Output, Run, Tool};

use crate::common;
use crate::error::{ToolError, ToolResult};

mod package;
#[cfg(windows)]
mod register;

/// How long a successful run stays on screen before its window closes.
const CLOSE_DELAY: Duration = Duration::from_secs(3);
/// How long a failure, or a result meant to be read, stays on screen.
const READ_DELAY: Duration = Duration::from_secs(30);
/// File arguments shown in the Headless line before it is abbreviated.
const SHOWN_INPUTS: usize = 4;

fn help() {
    println!(
        r#"just context — Put JustTools in the Windows File Explorer context menu.

Usage:
  just context install [--classic]
  just context uninstall
  just context status

Right-click a video, audio file, image, PDF, SVG, JSON file, document, or
folder and choose JustTools. Every entry opens a console window that shows the
equivalent Headless command, runs it, and then closes by itself; any key keeps
it open. Entries ending in "…" open the tool's console launcher instead.

install      Register the menu for the current user; no elevation is needed.
             On Windows 11 the top-level menu is used when a signed
             justtools-shell.msix sits beside just.exe or Developer Mode is on.
             Otherwise, and on Windows 10, the menu is registered under
             "Show more options".
  --classic  Always register under "Show more options".
uninstall    Remove every registration made by install.
status       Show which registration is active.

Sources are kept unless an entry says "in place". JustWebP and JustAVIF write
beside the source and keep it. Nothing is saved between runs."#
    );
}

fn usage(message: impl Into<String>) -> ToolError {
    ToolError::usage("just", message)
}

pub fn run(args: Vec<OsString>) -> ToolResult {
    let mut args = args.into_iter();
    let command = args
        .next()
        .map(|value| value.to_string_lossy().into_owned());
    let rest: Vec<OsString> = args.collect();
    match command.as_deref() {
        None | Some("-h" | "--help" | "help") => {
            help();
            Ok(())
        }
        Some("run") => run_entry(rest),
        Some("package") => write_package(rest),
        Some("install") => {
            let classic = match rest.as_slice() {
                [] => false,
                [flag] if flag == "--classic" => true,
                _ => return Err(usage("context install accepts only --classic")),
            };
            install(classic)
        }
        Some(name @ ("uninstall" | "status" | "refresh")) => {
            if !rest.is_empty() {
                return Err(usage(format!("context {name} does not take arguments")));
            }
            match name {
                "status" => status(),
                "uninstall" => uninstall(),
                _ => refresh(),
            }
        }
        Some(other) => Err(usage(format!("unknown context command: {other}"))),
    }
}

/// Write the package layout a release signs: `just context package --output
/// DIR [--publisher SUBJECT]`. The publisher must equal the certificate's
/// subject, so the release workflow supplies it.
fn write_package(args: Vec<OsString>) -> ToolResult {
    let mut output = None;
    let mut publisher = package::DEFAULT_PUBLISHER.to_owned();
    let mut index = 0;
    while index < args.len() {
        match args[index].to_string_lossy().as_ref() {
            "-o" | "--output" => {
                index += 1;
                output = args.get(index).map(PathBuf::from);
            }
            "--publisher" => {
                publisher = common::option_value("just", &args, &mut index, "--publisher")?;
            }
            other => return Err(usage(format!("unknown context package option: {other}"))),
        }
        index += 1;
    }
    let output = output.ok_or_else(|| usage("context package needs --output DIR"))?;
    package::write_layout(&output, &publisher)?;
    println!(
        "just: wrote the context-menu package layout to {}",
        output.display()
    );
    Ok(())
}

#[cfg(not(windows))]
fn windows_only() -> ToolError {
    ToolError::new(
        "just",
        "the context menu is a Windows File Explorer feature",
    )
}

#[cfg(not(windows))]
fn install(_classic: bool) -> ToolResult {
    Err(windows_only())
}

#[cfg(not(windows))]
fn uninstall() -> ToolResult {
    Err(windows_only())
}

#[cfg(not(windows))]
fn status() -> ToolResult {
    Err(windows_only())
}

#[cfg(windows)]
const TOP_LEVEL_HINT: &str = "For the top-level Windows 11 menu, turn on Developer Mode (Settings > System > For developers) and run `just context install` again.";

#[cfg(windows)]
fn install(classic: bool) -> ToolResult {
    let (tier, reason) = register::install(classic)?;
    match tier {
        register::Tier::Package => {
            println!("just: JustTools is in the Windows 11 context menu.");
        }
        register::Tier::Classic => {
            let status = register::status()?;
            if status.windows_11 {
                println!(
                    "just: JustTools is in the context menu under \"Show more options\" (or Shift+right-click)."
                );
            } else {
                println!("just: JustTools is in the context menu.");
            }
            if let Some(reason) = reason {
                println!("just: {reason}.");
                if status.windows_11 && !status.developer_mode && !status.signed_package {
                    println!("just: {TOP_LEVEL_HINT}");
                }
            }
        }
    }
    println!("just: right-click a video, image, PDF, or folder to use it.");
    Ok(())
}

#[cfg(windows)]
fn uninstall() -> ToolResult {
    register::uninstall()?;
    println!("just: JustTools was removed from the context menu.");
    Ok(())
}

#[cfg(windows)]
fn status() -> ToolResult {
    let status = register::status()?;
    match (&status.package, status.classic) {
        (Some(version), _) => {
            println!("Context menu: installed in the Windows 11 menu (package {version})")
        }
        (None, true) if status.windows_11 => {
            println!("Context menu: installed under \"Show more options\"")
        }
        (None, true) => println!("Context menu: installed"),
        (None, false) => println!("Context menu: not installed (run `just context install`)"),
    }
    if let Some(owner) = status.owner.as_deref().filter(|_| !status.owned) {
        println!(
            "Registered by: {} (another JustTools copy)",
            owner.display()
        );
    }
    println!(
        "Extension:    {}{}",
        status.library.display(),
        if status.library.is_file() {
            ""
        } else {
            " (missing)"
        }
    );
    if status.windows_11 {
        println!(
            "Top-level:    {}",
            if status.signed_package {
                "available (signed package present)"
            } else if status.developer_mode {
                "available (Developer Mode is on)"
            } else {
                "needs a signed justtools-shell.msix or Developer Mode"
            }
        );
    }
    Ok(())
}

/// Re-register a menu that is already installed so it matches the build now
/// on disk; a menu that was never installed stays absent. `just install` runs
/// this after replacing the files.
#[cfg(windows)]
fn refresh() -> ToolResult {
    match register::ownership()? {
        None => {
            if register::library_present() {
                println!(
                    "just: add the File Explorer right-click menu with `just context install`."
                );
            }
            return Ok(());
        }
        // Another copy of JustTools registered the menu; it is not ours to move.
        Some(false) => return Ok(()),
        Some(true) => {}
    }
    let status = register::status()?;
    if status.package.is_none() && !status.classic {
        return Ok(());
    }
    if !status.library.is_file() {
        // The new build ships no extension, so the old entry would point at
        // nothing.
        register::uninstall()?;
        println!("just: removed the File Explorer context menu; this build has no extension.");
        return Ok(());
    }
    register::install(status.package.is_none())?;
    println!("just: refreshed the File Explorer context menu.");
    Ok(())
}

#[cfg(not(windows))]
fn refresh() -> ToolResult {
    Ok(())
}

/// One headless run of a tool.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Invocation {
    directory: PathBuf,
    args: Vec<OsString>,
}

fn parent_of(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// A path as the tool should receive it when running inside `directory`.
///
/// Files in that folder go by name, which keeps the tool's own progress lines
/// short; a name that could be mistaken for an option is anchored with `.`.
fn argument(path: &Path, directory: &Path) -> OsString {
    match path.file_name() {
        Some(name) if parent_of(path) == directory => {
            if name.to_string_lossy().starts_with('-') {
                Path::new(".").join(name).into_os_string()
            } else {
                name.to_owned()
            }
        }
        _ => path.as_os_str().to_owned(),
    }
}

/// Group paths by parent folder, keeping first-seen order.
fn by_folder(paths: &[PathBuf]) -> Vec<(PathBuf, Vec<PathBuf>)> {
    let mut groups: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();
    for path in paths {
        let parent = parent_of(path);
        match groups.iter_mut().find(|(folder, _)| *folder == parent) {
            Some((_, files)) => files.push(path.clone()),
            None => groups.push((parent, vec![path.clone()])),
        }
    }
    groups
}

fn accepted(tool: Tool, paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|path| {
            if matches!(tool, Tool::Zip | Tool::Paste) {
                path.is_dir()
            } else {
                menu::extension(&path.to_string_lossy())
                    .is_some_and(|extension| tool.accepts(&extension))
            }
        })
        .cloned()
        .collect()
}

/// Turn a headless entry and the selection into the runs it stands for.
fn plan(action: &Action, tool: Tool, paths: &[PathBuf]) -> Result<Vec<Invocation>, String> {
    let inputs = accepted(tool, paths);
    if inputs.len() < action.minimum.max(1) as usize {
        return Err(if inputs.is_empty() {
            format!(
                "nothing in the selection is something {} reads",
                tool.name()
            )
        } else {
            format!(
                "{} needs at least {} files here",
                tool.name(),
                action.minimum
            )
        });
    }
    let preset = || action.args.iter().map(OsString::from);
    let invocation = |directory: &Path, files: &[PathBuf], output: bool| Invocation {
        directory: directory.to_path_buf(),
        args: preset()
            .chain(
                output
                    .then(|| ["--output", "."].map(OsString::from))
                    .into_iter()
                    .flatten(),
            )
            .chain(files.iter().map(|file| argument(file, directory)))
            .collect(),
    };
    if tool == Tool::Paste {
        // The folder is where the download lands, not something to read.
        return Ok(inputs
            .iter()
            .map(|folder| Invocation {
                directory: folder.clone(),
                args: preset().collect(),
            })
            .collect());
    }
    Ok(if action.per_file || tool == Tool::Zip {
        inputs
            .iter()
            .map(|file| invocation(&parent_of(file), std::slice::from_ref(file), false))
            .collect()
    } else if action.output == Output::SourceFolder {
        by_folder(&inputs)
            .iter()
            .map(|(folder, files)| invocation(folder, files, true))
            .collect()
    } else {
        vec![invocation(&parent_of(&inputs[0]), &inputs, false)]
    })
}

/// The command as a terminal user would type it, abbreviated once the file
/// list stops being readable.
fn headless_line(tool: Tool, action: &Action, invocation: &Invocation) -> String {
    let fixed = invocation.args.len().min(
        action.args.len()
            + if action.output == Output::SourceFolder {
                2
            } else {
                0
            },
    );
    let (options, inputs) = invocation.args.split_at(fixed);
    let mut parts = vec![tool.command().to_owned()];
    parts.extend(
        options
            .iter()
            .chain(inputs.iter().take(SHOWN_INPUTS))
            .map(|value| crate::launcher::quote(&value.to_string_lossy())),
    );
    if inputs.len() > SHOWN_INPUTS {
        parts.push(format!("… (+{} more)", inputs.len() - SHOWN_INPUTS));
    }
    parts.join(" ")
}

fn interactive() -> bool {
    common::stdin_is_terminal() && common::stdout_is_terminal()
}

fn set_title(title: &str) {
    if interactive() {
        crossterm::execute!(io::stdout(), crossterm::terminal::SetTitle(title)).ok();
    }
}

fn enter(directory: &Path) -> ToolResult {
    std::env::set_current_dir(directory).map_err(|error| {
        ToolError::new(
            "just",
            format!("cannot open {}: {error}", directory.display()),
        )
    })
}

/// What a finished entry leaves for the window to do.
enum Outcome {
    /// Nothing ran, or a full-screen console already had the last word.
    Closed,
    Ran {
        failed: bool,
    },
}

fn report(error: &ToolError) {
    eprintln!("{}: {}", error.tool(), error.message());
}

fn run_headless(action: &Action, tool: Tool, paths: &[PathBuf]) -> ToolResult<Outcome> {
    let invocations =
        plan(action, tool, paths).map_err(|message| ToolError::new("just", message))?;
    set_title(&format!("JustTools · {}", tool.name()));
    let mut failed = false;
    let mut cancelled = 0;
    for (index, invocation) in invocations.iter().enumerate() {
        if index > 0 {
            println!();
        }
        let command = headless_line(tool, action, invocation);
        let folder = invocation.directory.display();
        if interactive() {
            println!("{} {}", "Headless:".dark_grey(), command.magenta().bold());
            println!(
                "{} {folder}
",
                "Folder:  ".dark_grey()
            );
        } else {
            println!(
                "Headless: {command}
Folder:   {folder}
"
            );
        }
        enter(&invocation.directory)?;
        match crate::commands::dispatch_headless(tool.command(), invocation.args.clone()) {
            Ok(()) => {}
            Err(error) if error.exit_code() == 130 => cancelled += 1,
            Err(error) => {
                report(&error);
                failed = true;
            }
        }
    }
    Ok(if !failed && cancelled == invocations.len() {
        Outcome::Closed
    } else {
        Outcome::Ran { failed }
    })
}

/// Run what a launcher or the tool browser configured.
fn run_configured(command: &str, args: Option<Vec<OsString>>) -> Outcome {
    let Some(args) = args else {
        return Outcome::Closed;
    };
    match crate::commands::dispatch_headless(command, args) {
        Ok(()) => Outcome::Ran { failed: false },
        Err(error) if error.exit_code() == 130 => Outcome::Closed,
        Err(error) => {
            report(&error);
            Outcome::Ran { failed: true }
        }
    }
}

fn needs_terminal() -> ToolError {
    ToolError::new(
        "just",
        "this context-menu entry opens a console and needs a terminal",
    )
}

fn folder_of(paths: &[PathBuf]) -> PathBuf {
    match paths.first() {
        Some(path) if path.is_dir() => path.clone(),
        Some(path) => parent_of(path),
        None => PathBuf::from("."),
    }
}

/// The launcher's input row for a selection: names inside the working folder,
/// quoted wherever the row's `;` separator or quote handling would split them.
fn launcher_inputs(paths: &[PathBuf], directory: &Path) -> String {
    paths
        .iter()
        .map(|path| {
            let text = argument(path, directory).to_string_lossy().into_owned();
            if text.contains([';', '\'', ' ']) {
                format!("\"{text}\"")
            } else {
                text
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn perform(action: &Action, paths: &[PathBuf]) -> ToolResult<Outcome> {
    let Some(tool) = action.tool else {
        if !interactive() {
            return Err(needs_terminal());
        }
        enter(&folder_of(paths))?;
        set_title("JustTools");
        let Some(command) = crate::selector::pick()? else {
            return Ok(Outcome::Closed);
        };
        if !crate::launcher::supports(command) {
            // Live dashboards own the screen until they exit.
            crate::commands::dispatch(command, Vec::new())?;
            return Ok(Outcome::Closed);
        }
        return Ok(run_configured(command, crate::launcher::run(command)?));
    };
    match action.run {
        Run::Headless => run_headless(action, tool, paths),
        Run::Launcher | Run::LauncherHere => {
            if !interactive() {
                return Err(needs_terminal());
            }
            let directory = folder_of(paths);
            enter(&directory)?;
            set_title(&format!("JustTools · {}", tool.name()));
            let configured = if action.run == Run::Launcher {
                let inputs = launcher_inputs(&accepted(tool, paths), &directory);
                crate::launcher::run_with_input(tool.command(), &inputs)?
            } else {
                crate::launcher::run(tool.command())?
            };
            Ok(run_configured(tool.command(), configured))
        }
        Run::Browse => unreachable!("browse entries carry no tool"),
    }
}

fn read_list(path: &Path) -> ToolResult<Vec<PathBuf>> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        ToolError::new(
            "just",
            format!("cannot read the selection {}: {error}", path.display()),
        )
    })?;
    Ok(text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect())
}

/// `just context run <entry> [--list FILE] [path ...]`
fn run_entry(args: Vec<OsString>) -> ToolResult {
    let mut id = None;
    let mut list = None;
    let mut paths = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--list" {
            index += 1;
            list = Some(
                args.get(index)
                    .map(PathBuf::from)
                    .ok_or_else(|| usage("--list needs a file"))?,
            );
        } else if id.is_none() {
            id = Some(args[index].to_string_lossy().into_owned());
        } else {
            paths.push(PathBuf::from(&args[index]));
        }
        index += 1;
    }
    let id = id.ok_or_else(|| usage("context run needs a menu entry"))?;
    if let Some(list) = list {
        let listed = read_list(&list);
        // The list is a one-run handover from Explorer, not user data.
        std::fs::remove_file(&list).ok();
        paths.extend(listed?);
    }
    let action =
        menu::action(&id).ok_or_else(|| usage(format!("unknown context-menu entry: {id}")))?;
    let outcome = perform(&action, &paths);
    if !interactive() {
        return match outcome? {
            Outcome::Ran { failed: true } => {
                Err(ToolError::new("just", "the context-menu entry failed"))
            }
            _ => Ok(()),
        };
    }
    let failed = match outcome {
        Ok(Outcome::Closed) => return Ok(()),
        Ok(Outcome::Ran { failed }) => failed,
        Err(error) if error.exit_code() == 130 => return Ok(()),
        Err(error) => {
            report(&error);
            true
        }
    };
    hold(failed, action.hold);
    // The failure has been shown and acknowledged. A nonzero exit status
    // would only make Windows Terminal keep the finished tab open.
    Ok(())
}

fn key_pressed(timeout: Duration) -> Option<KeyCode> {
    if !event::poll(timeout).unwrap_or(false) {
        return None;
    }
    match event::read() {
        Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => Some(key.code),
        _ => None,
    }
}

fn closes(key: KeyCode) -> bool {
    matches!(key, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q'))
}

/// Keep the result readable, then close the window without being asked.
///
/// A plain success closes after a moment. A failure, or a result meant to be
/// read, gets longer. Enter closes at once and any other key stops the
/// countdown, so nothing disappears while it is being read.
fn hold(failed: bool, keep: bool) {
    let mut out = io::stdout();
    if crossterm::terminal::enable_raw_mode().is_err() {
        return;
    }
    let verdict = if failed {
        "Finished with errors.".red().bold()
    } else {
        "Done.".green().bold()
    };
    let delay = if failed || keep {
        READ_DELAY
    } else {
        CLOSE_DELAY
    };
    let deadline = Instant::now() + delay;
    let mut pinned = false;
    write!(out, "\r\n").ok();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let hint = format!(
            "Closing in {} s · Enter closes now · any other key keeps this window open",
            left.as_secs() + 1
        );
        write!(out, "\r{verdict} {}  ", hint.dark_grey()).ok();
        out.flush().ok();
        match key_pressed(left.min(Duration::from_millis(200))) {
            Some(key) if closes(key) => break,
            Some(_) => {
                pinned = true;
                break;
            }
            None => {}
        }
    }
    if pinned {
        let hint = format!("Press Enter to close this window.{:44}", "");
        write!(out, "\r{verdict} {}", hint.dark_grey()).ok();
        out.flush().ok();
        while !key_pressed(Duration::from_secs(3600)).is_some_and(closes) {}
    }
    crossterm::terminal::disable_raw_mode().ok();
    writeln!(out).ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(invocation: &Invocation) -> Vec<String> {
        invocation
            .args
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect()
    }

    fn paths(values: &[&str]) -> Vec<PathBuf> {
        values.iter().map(PathBuf::from).collect()
    }

    fn planned(id: &str, values: &[&str]) -> Vec<Invocation> {
        let action = menu::action(id).unwrap();
        plan(&action, action.tool.unwrap(), &paths(values)).unwrap()
    }

    #[test]
    fn a_resolution_entry_becomes_one_run_named_from_the_source_folder() {
        let runs = planned(
            "video.1080",
            &["clips/a.mov", "clips/b.mp4", "clips/notes.txt"],
        );
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].directory, PathBuf::from("clips"));
        assert_eq!(
            strings(&runs[0]),
            ["--resolution", "1080p", "a.mov", "b.mp4"]
        );
        let action = menu::action("video.1080").unwrap();
        assert_eq!(
            headless_line(Tool::Video, &action, &runs[0]),
            "justvideo --resolution 1080p a.mov b.mp4"
        );
    }

    #[test]
    fn source_consuming_converters_write_beside_each_source_and_keep_it() {
        let runs = planned("webp", &["one/a.png", "two/b.jpg", "one/c.png"]);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].directory, PathBuf::from("one"));
        assert_eq!(strings(&runs[0]), ["--output", ".", "a.png", "c.png"]);
        assert_eq!(runs[1].directory, PathBuf::from("two"));
        assert_eq!(strings(&runs[1]), ["--output", ".", "b.jpg"]);
    }

    #[test]
    fn single_input_operations_run_once_per_file_and_merge_runs_once() {
        let split = planned("pdf.split", &["docs/a.pdf", "docs/b.pdf"]);
        assert_eq!(split.len(), 2);
        assert_eq!(strings(&split[1]), ["split", "b.pdf"]);
        let merge = planned("pdf.merge", &["docs/a.pdf", "docs/b.pdf"]);
        assert_eq!(merge.len(), 1);
        assert_eq!(strings(&merge[0]), ["merge", "a.pdf", "b.pdf"]);

        let action = menu::action("pdf.merge").unwrap();
        let error = plan(&action, Tool::Pdf, &paths(&["docs/a.pdf"])).unwrap_err();
        assert!(error.contains("at least 2"));
    }

    #[test]
    fn pasting_runs_inside_the_folder_that_was_clicked() {
        let folder = tempfile::tempdir().unwrap();
        let action = menu::action("paste").unwrap();
        let runs = plan(&action, Tool::Paste, &[folder.path().to_path_buf()]).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].directory, folder.path());
        assert_eq!(strings(&runs[0]), ["--clipboard"]);
        assert_eq!(
            headless_line(Tool::Paste, &action, &runs[0]),
            "justpaste --clipboard"
        );
    }

    #[test]
    fn option_like_names_are_anchored_and_other_folders_stay_absolute() {
        let runs = planned("mp3", &["music/-intro.wav", "other/b.flac"]);
        let args = strings(&runs[0]);
        assert_eq!(PathBuf::from(&args[0]), Path::new(".").join("-intro.wav"));
        assert_eq!(PathBuf::from(&args[1]), PathBuf::from("other/b.flac"));
    }

    #[test]
    fn unreadable_selections_are_refused_and_long_ones_are_abbreviated() {
        let action = menu::action("optimize").unwrap();
        let error = plan(&action, Tool::Optimize, &paths(&["a.mp4"])).unwrap_err();
        assert!(error.contains("JustOptimize"));

        let many: Vec<String> = (0..9).map(|index| format!("p/{index}.png")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let runs = planned("resize.2560", &many);
        assert_eq!(
            headless_line(
                Tool::Resize,
                &menu::action("resize.2560").unwrap(),
                &runs[0]
            ),
            "justresize --max 2560 0.png 1.png 2.png 3.png … (+5 more)"
        );
    }

    #[test]
    fn launcher_input_quotes_names_the_row_would_split() {
        let row = launcher_inputs(
            &paths(&["clips/plain.mov", "clips/my clip.mov", "clips/a;b.mov"]),
            Path::new("clips"),
        );
        assert_eq!(row, r#"plain.mov; "my clip.mov"; "a;b.mov""#);
    }

    #[test]
    fn selection_lists_ignore_blank_lines_and_a_byte_order_mark() {
        let directory = tempfile::tempdir().unwrap();
        let list = directory.path().join("list.txt");
        std::fs::write(&list, "\u{feff}C:\\a b\\one.png\r\n\r\nC:\\two.png\n").unwrap();
        assert_eq!(
            read_list(&list).unwrap(),
            paths(&["C:\\a b\\one.png", "C:\\two.png"])
        );
    }
}
