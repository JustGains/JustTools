use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::catalog::{App, Platform, Source};
use super::detect::{self, Detection, LinuxManager};

#[derive(Clone, Debug)]
pub enum ActionKind {
    BootstrapWinget,
    BootstrapBrew,
    NativePackages {
        manager: LinuxManager,
        packages: Vec<String>,
    },
    ConfigureFlathub,
    Winget(&'static str),
    BrewFormula(&'static str),
    BrewCask(&'static str),
    Flatpak(&'static str),
    Script {
        url: &'static str,
        shell: &'static str,
        arguments: &'static [&'static str],
    },
    DotnetScript {
        url: &'static str,
        shell: &'static str,
        arguments: &'static [&'static str],
    },
}

#[derive(Clone, Debug)]
pub struct Action {
    pub label: String,
    pub app_ids: Vec<String>,
    pub kind: ActionKind,
    pub infrastructure: bool,
}

impl Action {
    pub fn preview(&self) -> String {
        match &self.kind {
            ActionKind::BootstrapWinget => concat!(
                "PowerShell: Install-Module Microsoft.WinGet.Client; ",
                "Repair-WinGetPackageManager -AllUsers"
            )
            .to_owned(),
            ActionKind::BootstrapBrew => concat!(
                "NONINTERACTIVE=1 /bin/bash -c ",
                "\"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\""
            )
            .to_owned(),
            ActionKind::NativePackages { manager, packages } => {
                let packages = packages.join(" ");
                match manager {
                    LinuxManager::Apt => format!("sudo apt-get install -y {packages}"),
                    LinuxManager::Dnf => format!("sudo dnf install -y {packages}"),
                    LinuxManager::Pacman => {
                        format!("sudo pacman -S --needed --noconfirm {packages}")
                    }
                    LinuxManager::Zypper => {
                        format!("sudo zypper --non-interactive install {packages}")
                    }
                    LinuxManager::Apk => format!("sudo apk add {packages}"),
                }
            }
            ActionKind::ConfigureFlathub => concat!(
                "flatpak remote-add --if-not-exists --user flathub ",
                "https://dl.flathub.org/repo/flathub.flatpakrepo"
            )
            .to_owned(),
            ActionKind::Winget(id) => format!(
                "winget install --id {id} --exact --source winget --silent ",
            ) + "--accept-package-agreements --accept-source-agreements --disable-interactivity",
            ActionKind::BrewFormula(name) => format!("brew install {name}"),
            ActionKind::BrewCask(name) => format!("brew install --cask {name}"),
            ActionKind::Flatpak(id) => {
                format!("flatpak install --user --noninteractive flathub {id}")
            }
            ActionKind::Script {
                url,
                shell,
                arguments,
            } => script_preview(url, shell, arguments),
            ActionKind::DotnetScript {
                url,
                shell,
                arguments,
            } => dotnet_paths().map_or_else(
                |_| script_preview(url, shell, arguments),
                |(installed, exposed)| {
                    format!(
                        "{} && ln -s {} {}",
                        script_preview(url, shell, arguments),
                        shell_quote(&installed),
                        shell_quote(&exposed)
                    )
                },
            ),
        }
    }
}

fn script_preview(url: &str, shell: &str, arguments: &[&str]) -> String {
    let suffix = if arguments.is_empty() {
        String::new()
    } else {
        format!(" -s -- {}", arguments.join(" "))
    };
    format!("curl -fsSL {url} | {shell}{suffix}")
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

#[derive(Clone, Debug)]
pub struct InstallPlan {
    pub app_ids: Vec<String>,
    pub dependency_ids: Vec<String>,
    pub actions: Vec<Action>,
}

pub fn build(
    platform: Platform,
    apps: &[App],
    requested_ids: &[String],
    detection: &Detection,
) -> Result<InstallPlan, String> {
    let by_id: HashMap<&str, &App> = apps.iter().map(|app| (app.id, app)).collect();
    let requested: HashSet<&str> = requested_ids.iter().map(String::as_str).collect();
    let mut included = HashSet::new();
    let mut visiting = HashSet::new();

    for id in requested_ids {
        include_app(id, &by_id, detection, &mut included, &mut visiting)?;
    }

    let app_ids = apps
        .iter()
        .filter(|app| included.contains(app.id))
        .map(|app| app.id.to_owned())
        .collect::<Vec<_>>();
    let dependency_ids = app_ids
        .iter()
        .filter(|id| !requested.contains(id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let required_ids = app_ids
        .iter()
        .filter_map(|id| by_id.get(id.as_str()).copied())
        .flat_map(|app| app.dependencies.iter().copied())
        .filter(|id| !detection.installed(id))
        .collect::<HashSet<_>>();
    let selected_apps = app_ids
        .iter()
        .filter_map(|id| by_id.get(id.as_str()).copied())
        .collect::<Vec<_>>();

    let needs_winget = selected_apps
        .iter()
        .any(|app| matches!(app.source, Source::Winget(_)));
    let needs_brew = selected_apps
        .iter()
        .any(|app| matches!(app.source, Source::BrewFormula(_) | Source::BrewCask(_)));
    let needs_flatpak = selected_apps
        .iter()
        .any(|app| matches!(app.source, Source::Flatpak(_)));
    let needs_script = selected_apps
        .iter()
        .any(|app| matches!(app.source, Source::Script { .. }));
    let needs_bash = selected_apps
        .iter()
        .any(|app| matches!(app.source, Source::Script { shell: "bash", .. }));

    let mut actions = Vec::new();
    if needs_winget && !detection.system.winget {
        actions.push(Action {
            label: "Install or repair WinGet".into(),
            app_ids: Vec::new(),
            kind: ActionKind::BootstrapWinget,
            infrastructure: true,
        });
    }
    if needs_brew && !detection.system.brew {
        if platform != Platform::MacOs {
            return Err("Homebrew bootstrap is currently supported only on macOS".into());
        }
        if !detection.system.curl {
            return Err(
                "Homebrew needs curl, but curl is not available on this macOS system".into(),
            );
        }
        actions.push(Action {
            label: "Install Homebrew".into(),
            app_ids: Vec::new(),
            kind: ActionKind::BootstrapBrew,
            infrastructure: true,
        });
    }

    let manager = detection.system.linux_manager;
    let mut native_packages = BTreeSet::new();
    let mut native_app_ids = Vec::new();
    for app in &selected_apps {
        if let Source::Native(packages) = app.source {
            let manager = manager.ok_or_else(|| {
                format!(
                    "{} needs a supported Linux package manager (apt, dnf, pacman, zypper, or apk)",
                    app.name
                )
            })?;
            let packages = manager.packages(packages);
            if packages.is_empty() {
                return Err(format!(
                    "{} is not packaged for the detected Linux distribution",
                    app.name
                ));
            }
            native_packages.extend(packages.iter().map(|package| (*package).to_owned()));
            native_app_ids.push(app.id.to_owned());
        }
    }
    if needs_flatpak && !detection.system.flatpak {
        manager.ok_or_else(|| {
            "Flatpak is missing and no supported Linux package manager can install it".to_owned()
        })?;
        native_packages.insert("flatpak".into());
    }
    if needs_script && !detection.system.curl {
        if platform != Platform::Linux {
            return Err("an official installer needs curl, but curl is unavailable".into());
        }
        manager.ok_or_else(|| {
            "curl is missing and no supported Linux package manager can install it".to_owned()
        })?;
        native_packages.insert("curl".into());
    }
    if needs_bash && !detection.system.bash {
        manager.ok_or_else(|| {
            "bash is missing and no supported Linux package manager can install it".to_owned()
        })?;
        native_packages.insert("bash".into());
    }
    if !native_packages.is_empty() {
        actions.push(Action {
            label: if native_app_ids.is_empty() {
                "Install installer prerequisites".into()
            } else {
                "Install Linux packages".into()
            },
            app_ids: native_app_ids,
            kind: ActionKind::NativePackages {
                manager: manager.expect("native packages require a manager"),
                packages: native_packages.into_iter().collect(),
            },
            infrastructure: true,
        });
    }
    if needs_flatpak && (!detection.system.flatpak || !detection.system.flathub) {
        actions.push(Action {
            label: "Configure Flathub".into(),
            app_ids: Vec::new(),
            kind: ActionKind::ConfigureFlathub,
            infrastructure: true,
        });
    }

    for app in selected_apps {
        let kind = match app.source {
            Source::Winget(id) => ActionKind::Winget(id),
            Source::BrewFormula(name) => ActionKind::BrewFormula(name),
            Source::BrewCask(name) => ActionKind::BrewCask(name),
            Source::Flatpak(id) => ActionKind::Flatpak(id),
            Source::Script {
                url,
                shell,
                arguments,
            } if app.id == "dotnet" => ActionKind::DotnetScript {
                url,
                shell,
                arguments,
            },
            Source::Script {
                url,
                shell,
                arguments,
            } => ActionKind::Script {
                url,
                shell,
                arguments,
            },
            Source::Native(_) => continue,
        };
        actions.push(Action {
            label: format!("Install {}", app.name),
            app_ids: vec![app.id.to_owned()],
            kind,
            infrastructure: required_ids.contains(app.id),
        });
    }

    Ok(InstallPlan {
        app_ids,
        dependency_ids,
        actions,
    })
}

fn include_app(
    id: &str,
    by_id: &HashMap<&str, &App>,
    detection: &Detection,
    included: &mut HashSet<String>,
    visiting: &mut HashSet<String>,
) -> Result<(), String> {
    let app = by_id
        .get(id)
        .copied()
        .ok_or_else(|| format!("unknown or unavailable app id: {id}"))?;
    if detection.installed(id) || included.contains(id) {
        return Ok(());
    }
    if !visiting.insert(id.to_owned()) {
        return Err(format!("dependency cycle involving {id}"));
    }
    for dependency in app.dependencies {
        include_app(dependency, by_id, detection, included, visiting)?;
    }
    visiting.remove(id);
    included.insert(id.to_owned());
    Ok(())
}

#[derive(Debug, Default)]
pub struct ExecutionReport {
    pub completed_app_ids: Vec<String>,
    pub failed: Vec<(String, String)>,
}

pub fn execute(plan: &InstallPlan) -> Result<ExecutionReport, String> {
    let mut report = ExecutionReport::default();
    println!(
        "\nJustReady will run {} installation step(s).",
        plan.actions.len()
    );
    for (index, action) in plan.actions.iter().enumerate() {
        println!("\n[{}/{}] {}", index + 1, plan.actions.len(), action.label);
        println!("  $ {}", action.preview());
        let result = execute_action(&action.kind);
        match result {
            Ok(()) => report
                .completed_app_ids
                .extend(action.app_ids.iter().cloned()),
            Err(error) if action.infrastructure => {
                return Err(format!("{} failed: {error}", action.label));
            }
            Err(error) => report.failed.push((action.label.clone(), error)),
        }
    }
    Ok(report)
}

fn execute_action(kind: &ActionKind) -> Result<(), String> {
    match kind {
        ActionKind::BootstrapWinget => {
            let shell = if detect::command_exists("pwsh") {
                "pwsh"
            } else {
                "powershell"
            };
            run(Command::new(shell).args([
                "-NoLogo",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                concat!(
                    "$ProgressPreference='SilentlyContinue'; ",
                    "Install-PackageProvider -Name NuGet -Force | Out-Null; ",
                    "Install-Module -Name Microsoft.WinGet.Client -Force -Repository PSGallery; ",
                    "Repair-WinGetPackageManager -AllUsers"
                ),
            ]))
        }
        ActionKind::BootstrapBrew => {
            run(Command::new("/bin/bash").env("NONINTERACTIVE", "1").args([
                "-c",
                "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)",
            ]))
        }
        ActionKind::NativePackages { manager, packages } => {
            let mut arguments = match manager {
                LinuxManager::Apt => vec!["install".into(), "-y".into()],
                LinuxManager::Dnf => vec!["install".into(), "-y".into()],
                LinuxManager::Pacman => {
                    vec!["-S".into(), "--needed".into(), "--noconfirm".into()]
                }
                LinuxManager::Zypper => {
                    vec!["--non-interactive".into(), "install".into()]
                }
                LinuxManager::Apk => vec!["add".into()],
            };
            arguments.extend(packages.iter().cloned());
            run_privileged(manager.command(), &arguments)
        }
        ActionKind::ConfigureFlathub => run(Command::new("flatpak").args([
            "remote-add",
            "--if-not-exists",
            "--user",
            "flathub",
            "https://dl.flathub.org/repo/flathub.flatpakrepo",
        ])),
        ActionKind::Winget(id) => run(Command::new("winget").args([
            "install",
            "--id",
            id,
            "--exact",
            "--source",
            "winget",
            "--silent",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--disable-interactivity",
        ])),
        ActionKind::BrewFormula(name) => run(brew_command()?.args(["install", name])),
        ActionKind::BrewCask(name) => run(brew_command()?.args(["install", "--cask", name])),
        ActionKind::Flatpak(id) => run(Command::new("flatpak").args([
            "install",
            "--user",
            "--noninteractive",
            "flathub",
            id,
        ])),
        ActionKind::Script {
            url,
            shell,
            arguments,
        } => run_script(url, shell, arguments),
        ActionKind::DotnetScript {
            url,
            shell,
            arguments,
        } => {
            run_script(url, shell, arguments)?;
            expose_dotnet()
        }
    }
}

fn run(command: &mut Command) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("could not start command: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(status.code().map_or_else(
            || "command was terminated".into(),
            |code| format!("exit code {code}"),
        ))
    }
}

fn run_privileged(program: &str, arguments: &[String]) -> Result<(), String> {
    let root = Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "0");
    if root {
        run(Command::new(program).args(arguments))
    } else if detect::command_exists("sudo") {
        run(Command::new("sudo").arg(program).args(arguments))
    } else {
        Err(format!(
            "{program} needs root access, but sudo is unavailable"
        ))
    }
}

fn brew_command() -> Result<Command, String> {
    if detect::command_exists("brew") {
        return Ok(Command::new("brew"));
    }
    for path in [
        "/opt/homebrew/bin/brew",
        "/usr/local/bin/brew",
        "/home/linuxbrew/.linuxbrew/bin/brew",
    ] {
        if Path::new(path).is_file() {
            return Ok(Command::new(path));
        }
    }
    Err("Homebrew was installed but its executable could not be found".into())
}

fn run_script(url: &str, shell: &str, arguments: &[&str]) -> Result<(), String> {
    let temporary = tempfile::Builder::new()
        .prefix("justready-")
        .suffix(".sh")
        .tempfile()
        .map_err(|error| format!("cannot create temporary installer: {error}"))?;
    let path = temporary.path().to_path_buf();
    run(Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--show-error",
            "--progress-bar",
            "--output",
        ])
        .arg(&path)
        .arg(url))?;
    run(Command::new(shell).arg(&path).args(arguments))
}

fn dotnet_paths() -> Result<(PathBuf, PathBuf), String> {
    let home = directories::BaseDirs::new()
        .ok_or_else(|| "cannot locate the home directory for .NET".to_owned())?
        .home_dir()
        .to_path_buf();
    let installed = home.join(".dotnet/dotnet");
    let executable_directory = std::env::current_exe()
        .map_err(|error| format!("cannot locate JustReady to expose dotnet: {error}"))?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot locate the JustReady executable directory".to_owned())?;
    Ok((installed, executable_directory.join("dotnet")))
}

fn expose_dotnet() -> Result<(), String> {
    let (installed, exposed) = dotnet_paths()?;
    if !installed.is_file() {
        return Err(format!(
            "the official installer completed without {}",
            installed.display()
        ));
    }
    if fs::symlink_metadata(&exposed).is_ok() {
        if crate::common::same_path(&installed, &exposed) {
            return Ok(());
        }
        return Err(format!(
            "refusing to replace existing {}",
            exposed.display()
        ));
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&installed, &exposed).map_err(|error| {
            format!(
                "installed .NET but could not expose {} beside JustReady: {error}",
                exposed.display()
            )
        })
    }
    #[cfg(not(unix))]
    {
        Err("the .NET user installer is supported only on Unix systems".into())
    }
}

pub fn names_for_ids<'a>(apps: &'a [App], ids: &[String]) -> Vec<&'a str> {
    ids.iter()
        .filter_map(|id| apps.iter().find(|app| app.id == id).map(|app| app.name))
        .collect()
}

pub fn preview_lines(plan: &InstallPlan) -> Vec<String> {
    plan.actions
        .iter()
        .map(|action| format!("{}\n    {}", action.label, action.preview()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ready::catalog;
    use crate::commands::ready::detect::SystemState;

    #[test]
    fn claude_adds_git_when_it_is_missing() {
        let apps = catalog::for_platform(Platform::Windows);
        let detection = Detection::test_with(
            SystemState {
                winget: true,
                ..SystemState::default()
            },
            &[],
        );
        let plan = build(
            Platform::Windows,
            &apps,
            &["claude-code".into()],
            &detection,
        )
        .unwrap();
        assert!(plan.app_ids.contains(&"git".into()));
        assert!(plan.dependency_ids.contains(&"git".into()));
    }

    #[test]
    fn installed_dependencies_are_not_reinstalled() {
        let apps = catalog::for_platform(Platform::Windows);
        let detection = Detection::test_with(
            SystemState {
                winget: true,
                ..SystemState::default()
            },
            &["git"],
        );
        let plan = build(
            Platform::Windows,
            &apps,
            &["claude-code".into()],
            &detection,
        )
        .unwrap();
        assert_eq!(plan.app_ids, ["claude-code"]);
    }

    #[test]
    fn required_apps_are_critical_even_when_explicitly_requested() {
        let apps = catalog::for_platform(Platform::Windows);
        let detection = Detection::test_with(
            SystemState {
                winget: true,
                ..SystemState::default()
            },
            &[],
        );
        let plan = build(
            Platform::Windows,
            &apps,
            &["git".into(), "claude-code".into()],
            &detection,
        )
        .unwrap();
        let git = plan
            .actions
            .iter()
            .find(|action| action.app_ids == ["git"])
            .unwrap();
        assert!(git.infrastructure);
    }

    #[test]
    fn missing_winget_is_bootstrapped_before_apps() {
        let apps = catalog::for_platform(Platform::Windows);
        let detection = Detection::test_with(SystemState::default(), &[]);
        let plan = build(Platform::Windows, &apps, &["git".into()], &detection).unwrap();
        assert!(matches!(
            plan.actions.first().map(|action| &action.kind),
            Some(ActionKind::BootstrapWinget)
        ));
    }

    #[test]
    fn linux_flatpak_setup_is_automatic() {
        let apps = catalog::for_platform(Platform::Linux);
        let detection = Detection::test_with(
            SystemState {
                linux_manager: Some(LinuxManager::Apt),
                curl: true,
                ..SystemState::default()
            },
            &[],
        );
        let plan = build(Platform::Linux, &apps, &["telegram".into()], &detection).unwrap();
        assert!(plan.actions.iter().any(|action| matches!(
            &action.kind,
            ActionKind::NativePackages { packages, .. } if packages.contains(&"flatpak".into())
        )));
        assert!(
            plan.actions
                .iter()
                .any(|action| matches!(action.kind, ActionKind::ConfigureFlathub))
        );
    }

    #[test]
    fn linux_dotnet_uses_the_official_lts_installer_and_exposes_the_cli() {
        let apps = catalog::for_platform(Platform::Linux);
        let detection = Detection::test_with(
            SystemState {
                curl: true,
                linux_manager: Some(LinuxManager::Apt),
                ..SystemState::default()
            },
            &[],
        );
        let plan = build(Platform::Linux, &apps, &["dotnet".into()], &detection).unwrap();
        assert!(plan.actions.iter().any(|action| matches!(
            &action.kind,
            ActionKind::NativePackages { packages, .. } if packages.contains(&"bash".into())
        )));
        let action = plan
            .actions
            .iter()
            .find(|action| action.app_ids == ["dotnet"])
            .unwrap();
        assert!(matches!(action.kind, ActionKind::DotnetScript { .. }));
        assert!(action.preview().contains("--channel LTS"));
        assert!(action.preview().contains("ln -s"));
    }
}
