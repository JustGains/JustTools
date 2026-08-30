use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use directories::BaseDirs;

use super::catalog::{App, NativePackages, Platform, Source};

const SCAN_TIMEOUT: Duration = Duration::from_secs(18);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinuxManager {
    Apt,
    Dnf,
    Pacman,
    Zypper,
    Apk,
}

impl LinuxManager {
    pub fn command(self) -> &'static str {
        match self {
            Self::Apt => "apt-get",
            Self::Dnf => "dnf",
            Self::Pacman => "pacman",
            Self::Zypper => "zypper",
            Self::Apk => "apk",
        }
    }

    pub fn packages(self, packages: NativePackages) -> &'static [&'static str] {
        match self {
            Self::Apt => packages.apt,
            Self::Dnf => packages.dnf,
            Self::Pacman => packages.pacman,
            Self::Zypper => packages.zypper,
            Self::Apk => packages.apk,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SystemState {
    pub winget: bool,
    pub brew: bool,
    pub flatpak: bool,
    pub flathub: bool,
    pub curl: bool,
    pub bash: bool,
    pub linux_manager: Option<LinuxManager>,
}

#[derive(Clone, Debug, Default)]
pub struct Detection {
    installed: HashMap<String, bool>,
    pub system: SystemState,
    pub warnings: Vec<String>,
}

impl Detection {
    pub fn installed(&self, id: &str) -> bool {
        self.installed.get(id).copied().unwrap_or(false)
    }

    pub fn installed_count(&self) -> usize {
        self.installed
            .values()
            .filter(|installed| **installed)
            .count()
    }

    #[cfg(test)]
    pub fn test_with(system: SystemState, installed: &[&str]) -> Self {
        Self {
            installed: installed
                .iter()
                .map(|id| ((*id).to_owned(), true))
                .collect(),
            system,
            warnings: Vec::new(),
        }
    }
}

struct CommandOutput {
    text: String,
    success: bool,
    timed_out: bool,
}

pub fn command_exists(command: &str) -> bool {
    let path = Path::new(command);
    if path.components().count() > 1 {
        return path.is_file();
    }

    let candidates = executable_candidates(command);
    env::var_os("PATH")
        .map(|path| {
            env::split_paths(&path).any(|directory| {
                candidates
                    .iter()
                    .any(|candidate| directory.join(candidate).is_file())
            })
        })
        .unwrap_or(false)
}

fn executable_candidates(command: &str) -> Vec<OsString> {
    #[cfg(windows)]
    {
        if Path::new(command).extension().is_some() {
            return vec![OsString::from(command)];
        }
        let extensions = env::var_os("PATHEXT")
            .map(|value| {
                value
                    .to_string_lossy()
                    .split(';')
                    .filter(|extension| !extension.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|extensions| !extensions.is_empty())
            .unwrap_or_else(|| vec![".COM".into(), ".EXE".into(), ".BAT".into(), ".CMD".into()]);
        let mut candidates = vec![OsString::from(command)];
        candidates.extend(
            extensions
                .into_iter()
                .map(|extension| OsString::from(format!("{command}{extension}"))),
        );
        candidates
    }
    #[cfg(not(windows))]
    {
        vec![OsString::from(command)]
    }
}

pub fn scan(platform: Platform, apps: &[App]) -> Detection {
    let mut detection = Detection::default();
    detection.system.winget = command_exists("winget");
    detection.system.brew = command_exists("brew");
    detection.system.flatpak = command_exists("flatpak");
    detection.system.curl = command_exists("curl");
    detection.system.bash = command_exists("bash");
    detection.system.linux_manager = detect_linux_manager();

    let mut package_inventory = String::new();
    match platform {
        Platform::Windows if detection.system.winget => {
            collect_inventory(
                &mut detection,
                &mut package_inventory,
                "winget",
                &[
                    "list",
                    "--accept-source-agreements",
                    "--disable-interactivity",
                ],
                "WinGet",
            );
        }
        Platform::MacOs if detection.system.brew => {
            collect_inventory(
                &mut detection,
                &mut package_inventory,
                "brew",
                &["list", "--formula", "--full-name"],
                "Homebrew formula",
            );
            collect_inventory(
                &mut detection,
                &mut package_inventory,
                "brew",
                &["list", "--cask", "--full-name"],
                "Homebrew cask",
            );
        }
        Platform::Linux => {
            if let Some(manager) = detection.system.linux_manager {
                let (command, arguments): (&str, &[&str]) = match manager {
                    LinuxManager::Apt => ("dpkg-query", &["-W", "-f=${binary:Package}\n"]),
                    LinuxManager::Dnf => ("rpm", &["-qa", "--qf", "%{NAME}\n"]),
                    LinuxManager::Pacman => ("pacman", &["-Qq"]),
                    LinuxManager::Zypper => {
                        ("zypper", &["--no-refresh", "search", "--installed-only"])
                    }
                    LinuxManager::Apk => ("apk", &["info"]),
                };
                collect_inventory(
                    &mut detection,
                    &mut package_inventory,
                    command,
                    arguments,
                    "system packages",
                );
            }
            if detection.system.flatpak {
                collect_inventory(
                    &mut detection,
                    &mut package_inventory,
                    "flatpak",
                    &["list", "--app", "--columns=application"],
                    "Flatpak",
                );
                let remotes = run_output("flatpak", &["remotes", "--columns=name"], SCAN_TIMEOUT);
                detection.system.flathub = remotes.success
                    && remotes
                        .text
                        .lines()
                        .any(|line| line.trim().eq_ignore_ascii_case("flathub"));
            }
        }
        _ => {}
    }

    let inventory = package_inventory.to_ascii_lowercase();
    for app in apps {
        let found_by_command = app.probes.iter().any(|probe| command_exists(probe));
        let found_by_package = match app.source {
            Source::Native(packages) => detection.system.linux_manager.is_some_and(|manager| {
                manager
                    .packages(packages)
                    .iter()
                    .any(|package| inventory_has(&inventory, package))
            }),
            source => source
                .package_key()
                .is_some_and(|package| inventory_has(&inventory, package)),
        };
        let found_by_display_name = inventory_display_name_prefixes(app.id)
            .iter()
            .any(|name| inventory_has_display_name(&inventory, name));
        let found_by_standard_path = standard_user_paths(app.id)
            .iter()
            .any(|relative| user_file_exists(relative));
        let found_by_macos_bundle = platform == Platform::MacOs
            && macos_bundle_names(app.id)
                .iter()
                .any(|name| macos_bundle_exists(name));
        detection.installed.insert(
            app.id.to_owned(),
            found_by_command
                || found_by_package
                || found_by_display_name
                || found_by_standard_path
                || found_by_macos_bundle,
        );
    }
    detection
}

fn inventory_has(inventory: &str, package: &str) -> bool {
    let package = package.to_ascii_lowercase();
    inventory.lines().any(|line| {
        line.trim().eq_ignore_ascii_case(&package)
            || line.split_whitespace().any(|word| {
                word.trim_matches(|character: char| {
                    !character.is_ascii_alphanumeric() && !matches!(character, '.' | '-' | '_')
                })
                .eq_ignore_ascii_case(&package)
            })
    })
}

fn inventory_display_name_prefixes(id: &str) -> &'static [&'static str] {
    match id {
        // WinGet uses ARP/MSIX identities rather than community package ids for
        // some apps installed by their own updaters or the Microsoft Store.
        "chrome" => &["google chrome"],
        "firefox" => &["mozilla firefox"],
        "everything" => &["everything"],
        "winmerge" => &["winmerge"],
        _ => &[],
    }
}

fn inventory_has_display_name(inventory: &str, name: &str) -> bool {
    inventory.lines().any(|line| {
        let line = line.trim_start();
        line.get(..name.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(name))
            && line
                .get(name.len()..)
                .and_then(|rest| rest.chars().next())
                .is_none_or(|character| !character.is_ascii_alphanumeric())
    })
}

fn standard_user_paths(id: &str) -> &'static [&'static str] {
    match id {
        "codex" => &[".local/bin/codex"],
        "claude-code" => &[".local/bin/claude"],
        "dotnet" => &[".dotnet/dotnet"],
        "zed" => &[".local/bin/zed"],
        "bun" => &[".bun/bin/bun"],
        "rustup" => &[".cargo/bin/rustup", ".cargo/bin/cargo"],
        _ => &[],
    }
}

fn user_file_exists(relative: &str) -> bool {
    let Some(base) = BaseDirs::new() else {
        return false;
    };
    let path = base.home_dir().join(relative);
    path.is_file() || (cfg!(windows) && path.with_extension("exe").is_file())
}

fn macos_bundle_names(id: &str) -> &'static [&'static str] {
    match id {
        "github-desktop" => &["GitHub Desktop.app"],
        "bitwarden" => &["Bitwarden.app"],
        "tailscale" => &["Tailscale.app"],
        "claude-desktop" => &["Claude.app"],
        "ollama" => &["Ollama.app"],
        "lm-studio" => &["LM Studio.app"],
        "zed" => &["Zed.app"],
        "vscode" => &["Visual Studio Code.app"],
        "cursor" => &["Cursor.app"],
        "warp" => &["Warp.app"],
        "docker" => &["Docker.app"],
        "dbeaver" => &["DBeaver.app"],
        "postman" => &["Postman.app"],
        "bruno" => &["Bruno.app"],
        "insomnia" => &["Insomnia.app"],
        "beekeeper" => &["Beekeeper Studio.app"],
        "telegram" => &["Telegram.app"],
        "notion" => &["Notion.app"],
        "discord" => &["Discord.app"],
        "slack" => &["Slack.app"],
        "signal" => &["Signal.app"],
        "zoom" => &["zoom.us.app"],
        "firefox" => &["Firefox.app"],
        "chrome" => &["Google Chrome.app"],
        "brave" => &["Brave Browser.app"],
        "keepassxc" => &["KeePassXC.app"],
        "vlc" => &["VLC.app"],
        "obs" => &["OBS.app"],
        "figma" => &["Figma.app"],
        "gimp" => &["GIMP.app"],
        "inkscape" => &["Inkscape.app"],
        _ => &[],
    }
}

fn macos_bundle_exists(name: &str) -> bool {
    if Path::new("/Applications").join(name).is_dir() {
        return true;
    }
    BaseDirs::new().is_some_and(|base| base.home_dir().join("Applications").join(name).is_dir())
}

fn detect_linux_manager() -> Option<LinuxManager> {
    [
        ("apt-get", LinuxManager::Apt),
        ("dnf", LinuxManager::Dnf),
        ("pacman", LinuxManager::Pacman),
        ("zypper", LinuxManager::Zypper),
        ("apk", LinuxManager::Apk),
    ]
    .into_iter()
    .find_map(|(command, manager)| command_exists(command).then_some(manager))
}

fn collect_inventory(
    detection: &mut Detection,
    inventory: &mut String,
    program: &str,
    arguments: &[&str],
    label: &str,
) {
    let output = run_output(program, arguments, SCAN_TIMEOUT);
    if output.success {
        inventory.push_str(&output.text);
        inventory.push('\n');
    } else if output.timed_out {
        detection.warnings.push(format!(
            "{label} inventory timed out; command checks were still used"
        ));
    } else {
        detection.warnings.push(format!(
            "{label} inventory was unavailable; command checks were still used"
        ));
    }
}

fn run_output(program: &str, arguments: &[&str], timeout: Duration) -> CommandOutput {
    let Ok(mut temporary) = tempfile::tempfile() else {
        return CommandOutput {
            text: String::new(),
            success: false,
            timed_out: false,
        };
    };
    let Ok(stdout) = temporary.try_clone() else {
        return CommandOutput {
            text: String::new(),
            success: false,
            timed_out: false,
        };
    };
    let child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return CommandOutput {
            text: String::new(),
            success: false,
            timed_out: false,
        };
    };

    let started = Instant::now();
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (Some(status), false),
            Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(40)),
            Ok(None) => {
                let _ = child.kill();
                break (child.wait().ok(), true);
            }
            Err(_) => break (None, false),
        }
    };

    let mut text = String::new();
    let _ = temporary.seek(SeekFrom::Start(0));
    let _ = temporary.read_to_string(&mut text);
    CommandOutput {
        text,
        success: status.is_some_and(|status| status.success()) && !timed_out,
        timed_out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_matching_is_exact_enough_for_table_inventory() {
        let inventory = "Name   Git.Git   2.50.0\norg.mozilla.firefox\n";
        assert!(inventory_has(inventory, "Git.Git"));
        assert!(inventory_has(inventory, "org.mozilla.firefox"));
        assert!(!inventory_has(inventory, "git"));
    }

    #[test]
    fn display_name_matching_handles_registry_ids_without_partial_names() {
        let inventory = "Google Chrome    ARP\\Machine\\X86\\Google Chrome\nDiscord PTB    ARP\\User\\DiscordPTB\n";
        assert!(inventory_has_display_name(inventory, "google chrome"));
        assert!(!inventory_has_display_name(inventory, "discord p"));
    }

    #[test]
    fn command_candidates_include_the_unmodified_name() {
        assert!(
            executable_candidates("justready")
                .iter()
                .any(|candidate| candidate == "justready")
        );
    }
}
