//! Cross-platform detection and confirmed installation of external programs.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::fs;
use std::io::{self, IsTerminal, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{ToolError, ToolResult};

/// Locate an executable either at an explicit path or on `PATH`.
pub fn find_executable(name: &str) -> Option<PathBuf> {
    find_executable_with(name, env::var_os("PATH"), env::var_os("PATHEXT"))
}

fn find_executable_with(
    name: &str,
    search_path: Option<OsString>,
    path_extensions: Option<OsString>,
) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }

    let explicit = is_explicit_path(name);
    let directories: Vec<PathBuf> = if explicit {
        vec![PathBuf::new()]
    } else {
        env::split_paths(&search_path.unwrap_or_default()).collect()
    };
    let extensions = executable_extensions(name, path_extensions);

    for directory in directories {
        for extension in &extensions {
            let candidate = if explicit {
                PathBuf::from(format!("{name}{extension}"))
            } else {
                directory.join(format!("{name}{extension}"))
            };
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn executable_extensions(name: &str, path_extensions: Option<OsString>) -> Vec<String> {
    #[cfg(windows)]
    {
        if Path::new(name).extension().is_some() {
            return vec![String::new()];
        }
        let extensions = path_extensions.unwrap_or_else(|| OsString::from(".COM;.EXE;.BAT;.CMD"));
        let mut values = vec![String::new()];
        values.extend(
            extensions
                .to_string_lossy()
                .split(';')
                .filter(|extension| !extension.is_empty())
                .map(str::to_owned),
        );
        values
    }
    #[cfg(not(windows))]
    {
        let _ = (name, path_extensions);
        vec![String::new()]
    }
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn is_explicit_path(name: &str) -> bool {
    name.contains('/') || name.contains('\\') || Path::new(name).is_absolute()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Dependency {
    Ffmpeg,
    Pngquant,
    Cwebp,
    Git,
}

impl Dependency {
    fn from_standard_name(name: &str) -> Option<Self> {
        match name {
            "ffmpeg" => Some(Self::Ffmpeg),
            "pngquant" => Some(Self::Pngquant),
            "cwebp" => Some(Self::Cwebp),
            "git" => Some(Self::Git),
            _ => None,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Ffmpeg => "FFmpeg and ffprobe",
            Self::Pngquant => "pngquant",
            Self::Cwebp => "cwebp",
            Self::Git => "Git",
        }
    }

    fn executables(self) -> &'static [&'static str] {
        match self {
            Self::Ffmpeg => &["ffmpeg", "ffprobe"],
            Self::Pngquant => &["pngquant"],
            Self::Cwebp => &["cwebp"],
            Self::Git => &["git"],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Platform {
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    fn current() -> Result<Self, BootstrapError> {
        if cfg!(target_os = "windows") {
            Ok(Self::Windows)
        } else if cfg!(target_os = "macos") {
            Ok(Self::MacOs)
        } else if cfg!(target_os = "linux") {
            Ok(Self::Linux)
        } else {
            Err(BootstrapError::UnsupportedPlatform(env::consts::OS.into()))
        }
    }
}

impl Display for Platform {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Windows => "Windows",
            Self::MacOs => "macOS",
            Self::Linux => "Linux",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Installer {
    Winget,
    Homebrew,
    Apt,
    Dnf,
    Pacman,
    VendorArchive,
}

impl Display for Installer {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Winget => "WinGet",
            Self::Homebrew => "Homebrew",
            Self::Apt => "APT",
            Self::Dnf => "DNF",
            Self::Pacman => "pacman",
            Self::VendorArchive => "verified vendor archive",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CommandSpec {
    program: PathBuf,
    args: Vec<OsString>,
    display: String,
}

impl CommandSpec {
    fn simple(program: PathBuf, display_program: &str, args: &[&str]) -> Self {
        Self {
            program,
            args: args.iter().map(OsString::from).collect(),
            display: std::iter::once(display_program)
                .chain(args.iter().copied())
                .map(quote_command_part)
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

fn quote_command_part(value: &str) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-._/:".contains(character))
    {
        value.into()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct InstallPlan {
    installer: Installer,
    commands: Vec<CommandSpec>,
}

impl InstallPlan {
    fn commands_for_display(&self) -> String {
        self.commands
            .iter()
            .map(|command| command.display.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug)]
enum BootstrapError {
    UnsupportedPlatform(String),
    ResolveOnlyMissing {
        requested: String,
        reason: String,
    },
    NoInstaller {
        dependency: Dependency,
        platform: Platform,
        guidance: String,
    },
    InstallIo {
        command: String,
        source: io::Error,
    },
    InstallFailed {
        command: String,
        exit_code: Option<i32>,
    },
    Interaction(io::Error),
}

impl BootstrapError {
    fn into_tool_error(self, tool: &str) -> ToolError {
        ToolError::new(tool, self.to_string())
    }
}

impl Display for BootstrapError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedPlatform(platform) => write!(
                formatter,
                "automatic dependency installation is not supported on {platform}"
            ),
            Self::ResolveOnlyMissing { requested, reason } => write!(
                formatter,
                "{requested} was not found; automatic installation is disabled {reason}"
            ),
            Self::NoInstaller {
                dependency,
                platform,
                guidance,
            } => write!(
                formatter,
                "cannot install {} on {platform}: {guidance}",
                dependency.display_name()
            ),
            Self::InstallIo { command, source } => {
                write!(
                    formatter,
                    "could not run dependency installer `{command}`: {source}"
                )
            }
            Self::InstallFailed { command, exit_code } => match exit_code {
                Some(code) => write!(
                    formatter,
                    "dependency installer `{command}` exited with status {code}"
                ),
                None => write!(
                    formatter,
                    "dependency installer `{command}` was terminated before returning a status"
                ),
            },
            Self::Interaction(source) => {
                write!(
                    formatter,
                    "could not read dependency confirmation: {source}"
                )
            }
        }
    }
}

impl std::error::Error for BootstrapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InstallIo { source, .. } | Self::Interaction(source) => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EnsureOutcome {
    AlreadyAvailable(PathBuf),
    Declined,
    NeedsInteractiveConfirmation { commands: String },
    Installed(PathBuf),
    InstalledButNotVisible { missing: Vec<&'static str> },
}

trait DependencyRunner {
    fn find(&mut self, program: &str) -> Option<PathBuf>;
    fn run(&mut self, command: &CommandSpec) -> io::Result<Option<i32>>;
}

trait DependencyInteraction {
    fn is_interactive(&self) -> bool;
    fn confirm(&mut self, prompt: &str) -> io::Result<bool>;
    fn print_line(&mut self, message: &str);
}

#[derive(Default)]
struct SystemRunner;

impl DependencyRunner for SystemRunner {
    fn find(&mut self, program: &str) -> Option<PathBuf> {
        find_executable(program)
    }

    fn run(&mut self, command: &CommandSpec) -> io::Result<Option<i32>> {
        Command::new(&command.program)
            .args(&command.args)
            .status()
            .map(|status| status.code())
    }
}

struct SystemInteraction {
    interactive: bool,
}

impl SystemInteraction {
    fn new() -> Self {
        Self {
            interactive: io::stdin().is_terminal() && io::stderr().is_terminal(),
        }
    }
}

impl DependencyInteraction for SystemInteraction {
    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn confirm(&mut self, prompt: &str) -> io::Result<bool> {
        eprint!("{prompt} [y/N] ");
        io::stderr().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        Ok(matches!(
            answer.trim().to_ascii_lowercase().as_str(),
            "y" | "yes"
        ))
    }

    fn print_line(&mut self, message: &str) {
        eprintln!("{message}");
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RequestPolicy {
    Install(Dependency),
    ResolveOnly(String),
}

const DEPENDENCY_OVERRIDES: [&str; 4] = ["FFMPEG_BIN", "PNGQUANT_BIN", "CWEBP_BIN", "GIT_BIN"];

fn request_policy(requested: &str) -> RequestPolicy {
    request_policy_with(requested, |name| env::var_os(name))
}

fn request_policy_with<F>(requested: &str, mut environment: F) -> RequestPolicy
where
    F: FnMut(&str) -> Option<OsString>,
{
    for variable in DEPENDENCY_OVERRIDES {
        if environment(variable).as_deref() == Some(OsStr::new(requested)) {
            return RequestPolicy::ResolveOnly(format!("because {variable} is set"));
        }
    }
    if is_explicit_path(requested) {
        return RequestPolicy::ResolveOnly("for explicit paths".into());
    }
    match Dependency::from_standard_name(requested) {
        Some(dependency) => RequestPolicy::Install(dependency),
        None => RequestPolicy::ResolveOnly("for custom executable names".into()),
    }
}

fn inspect_dependency<R: DependencyRunner>(
    dependency: Dependency,
    requested: &str,
    runner: &mut R,
) -> (Option<PathBuf>, Vec<&'static str>) {
    let mut requested_path = None;
    let mut missing = Vec::new();
    for executable in dependency.executables() {
        match runner.find(executable) {
            Some(path) => {
                if *executable == requested {
                    requested_path = Some(path);
                }
            }
            None => missing.push(*executable),
        }
    }
    (requested_path, missing)
}

fn build_install_plan<R: DependencyRunner>(
    dependency: Dependency,
    platform: Platform,
    runner: &mut R,
) -> Result<InstallPlan, BootstrapError> {
    match platform {
        Platform::Windows => windows_plan(dependency, runner),
        Platform::MacOs => macos_plan(dependency, runner),
        Platform::Linux => linux_plan(dependency, runner),
    }
}

fn windows_plan<R: DependencyRunner>(
    dependency: Dependency,
    runner: &mut R,
) -> Result<InstallPlan, BootstrapError> {
    if dependency == Dependency::Pngquant {
        let shell = runner
            .find("powershell.exe")
            .or_else(|| runner.find("pwsh"))
            .ok_or_else(|| BootstrapError::NoInstaller {
                dependency,
                platform: Platform::Windows,
                guidance: "PowerShell is required for the checksum-verified official archive. Download https://pngquant.org/pngquant-windows.zip and place pngquant.exe on PATH.".into(),
            })?;
        return Ok(pngquant_windows_plan(shell));
    }

    let package_id = match dependency {
        Dependency::Ffmpeg => "Gyan.FFmpeg",
        Dependency::Cwebp => "Google.Libwebp",
        Dependency::Git => "Git.Git",
        Dependency::Pngquant => unreachable!(),
    };
    let winget = runner
        .find("winget")
        .ok_or_else(|| BootstrapError::NoInstaller {
            dependency,
            platform: Platform::Windows,
            guidance: format!(
                "WinGet is required. Install Microsoft's App Installer, then run: winget install --id {package_id} --exact --source winget"
            ),
        })?;

    Ok(InstallPlan {
        installer: Installer::Winget,
        commands: vec![CommandSpec::simple(
            winget,
            "winget",
            &[
                "install",
                "--id",
                package_id,
                "--exact",
                "--source",
                "winget",
                "--accept-package-agreements",
                "--accept-source-agreements",
            ],
        )],
    })
}

fn pngquant_windows_plan(shell: PathBuf) -> InstallPlan {
    // This upstream URL is not versioned, so its pinned hash deliberately fails
    // closed when the vendor replaces the archive.
    const SCRIPT: &str = r#"& {
$ErrorActionPreference = 'Stop'
$version = '3.0.3'
$url = 'https://pngquant.org/pngquant-windows.zip'
$expected = 'BD0257AEECCFE446A4CD764927E26F8AF6051796F28ABED104307284107B120D'
$base = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) "JustTools\deps\pngquant\$version"
$archive = Join-Path ([IO.Path]::GetTempPath()) "justtools-pngquant-$([Guid]::NewGuid().ToString('N')).zip"
try {
  Invoke-WebRequest -Uri $url -OutFile $archive
  $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToUpperInvariant()
  if ($actual -ne $expected) { throw "pngquant archive hash mismatch: expected $expected, received $actual" }
  New-Item -ItemType Directory -Force -Path $base | Out-Null
  Expand-Archive -LiteralPath $archive -DestinationPath $base -Force
  $bin = Join-Path $base 'pngquant'
  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  $entries = @($userPath -split ';' | Where-Object { $_ } | ForEach-Object { $_.Trim().TrimEnd('\') })
  if ($entries -notcontains $bin.TrimEnd('\')) {
    [Environment]::SetEnvironmentVariable('Path', ((@($entries) + $bin) -join ';'), 'User')
  }
  $env:Path = "$bin;$env:Path"
  & (Join-Path $bin 'pngquant.exe') --version
} finally {
  if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }
}
}"#;
    let shell_name = shell
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("powershell.exe")
        .to_owned();
    let display_script = SCRIPT.replace('\'', "''");
    InstallPlan {
        installer: Installer::VendorArchive,
        commands: vec![CommandSpec {
            program: shell,
            args: vec!["-NoProfile".into(), "-Command".into(), SCRIPT.into()],
            display: format!("{shell_name} -NoProfile -Command '{display_script}'"),
        }],
    }
}

fn macos_plan<R: DependencyRunner>(
    dependency: Dependency,
    runner: &mut R,
) -> Result<InstallPlan, BootstrapError> {
    let package = package_for(Installer::Homebrew, dependency);
    let brew = runner
        .find("brew")
        .ok_or_else(|| BootstrapError::NoInstaller {
            dependency,
            platform: Platform::MacOs,
            guidance: format!(
                "Homebrew is required. Install it from https://brew.sh, then run: brew install {package}"
            ),
        })?;
    Ok(InstallPlan {
        installer: Installer::Homebrew,
        commands: vec![CommandSpec::simple(brew, "brew", &["install", package])],
    })
}

fn linux_plan<R: DependencyRunner>(
    dependency: Dependency,
    runner: &mut R,
) -> Result<InstallPlan, BootstrapError> {
    let (installer, manager, manager_name) = if let Some(manager) = runner.find("apt-get") {
        (Installer::Apt, manager, "apt-get")
    } else if let Some(manager) = runner.find("dnf") {
        (Installer::Dnf, manager, "dnf")
    } else if let Some(manager) = runner.find("pacman") {
        (Installer::Pacman, manager, "pacman")
    } else {
        return Err(BootstrapError::NoInstaller {
            dependency,
            platform: Platform::Linux,
            guidance: format!(
                "no supported package manager was found. Install one of these distro packages: apt `{}`, dnf `{}`, pacman `{}`",
                package_for(Installer::Apt, dependency),
                package_for(Installer::Dnf, dependency),
                package_for(Installer::Pacman, dependency),
            ),
        });
    };

    let package = package_for(installer, dependency);
    let mut args = match installer {
        Installer::Apt => vec!["install", "-y", package],
        Installer::Dnf => vec!["install", "-y", package],
        Installer::Pacman => vec!["-S", "--needed", "--noconfirm", package],
        _ => unreachable!(),
    };
    let command = if let Some(sudo) = runner.find("sudo") {
        args.insert(0, manager_name);
        CommandSpec::simple(sudo, "sudo", &args)
    } else {
        CommandSpec::simple(manager, manager_name, &args)
    };

    Ok(InstallPlan {
        installer,
        commands: vec![command],
    })
}

fn package_for(installer: Installer, dependency: Dependency) -> &'static str {
    match (installer, dependency) {
        (Installer::Homebrew, Dependency::Ffmpeg) => "ffmpeg",
        (Installer::Homebrew, Dependency::Pngquant) => "pngquant",
        (Installer::Homebrew, Dependency::Cwebp) => "webp",
        (Installer::Homebrew, Dependency::Git) => "git",
        (Installer::Apt, Dependency::Ffmpeg) => "ffmpeg",
        (Installer::Apt, Dependency::Pngquant) => "pngquant",
        (Installer::Apt, Dependency::Cwebp) => "webp",
        (Installer::Apt, Dependency::Git) => "git",
        (Installer::Dnf, Dependency::Ffmpeg) => "ffmpeg-free",
        (Installer::Dnf, Dependency::Pngquant) => "pngquant",
        (Installer::Dnf, Dependency::Cwebp) => "libwebp-tools",
        (Installer::Dnf, Dependency::Git) => "git",
        (Installer::Pacman, Dependency::Ffmpeg) => "ffmpeg",
        (Installer::Pacman, Dependency::Pngquant) => "pngquant",
        (Installer::Pacman, Dependency::Cwebp) => "libwebp-utils",
        (Installer::Pacman, Dependency::Git) => "git",
        _ => unreachable!("package requested for an installer without package names"),
    }
}

fn ensure_dependency<R: DependencyRunner, I: DependencyInteraction>(
    tool: &str,
    requested: &str,
    dependency: Dependency,
    platform: Platform,
    runner: &mut R,
    interaction: &mut I,
) -> Result<EnsureOutcome, BootstrapError> {
    let (requested_path, missing) = inspect_dependency(dependency, requested, runner);
    if missing.is_empty() {
        return Ok(EnsureOutcome::AlreadyAvailable(requested_path.expect(
            "standard dependency must contain the requested executable",
        )));
    }

    let plan = build_install_plan(dependency, platform, runner)?;
    interaction.print_line(&format!(
        "{tool}: missing {} ({})",
        dependency.display_name(),
        missing.join(", ")
    ));
    interaction.print_line(&format!("{tool}: installer: {}", plan.installer));
    if dependency == Dependency::Ffmpeg && plan.installer == Installer::Dnf {
        interaction.print_line(
            "justtools: Fedora's ffmpeg-free package may omit libx264, libmp3lame, or libaom-av1; the requested tool will verify its encoder after installation",
        );
    }
    for command in &plan.commands {
        interaction.print_line(&format!("{tool}: proposed command: {}", command.display));
    }

    if !interaction.is_interactive() {
        return Ok(EnsureOutcome::NeedsInteractiveConfirmation {
            commands: plan.commands_for_display(),
        });
    }
    if !interaction
        .confirm(&format!(
            "{tool}: install {} now?",
            dependency.display_name()
        ))
        .map_err(BootstrapError::Interaction)?
    {
        return Ok(EnsureOutcome::Declined);
    }

    for command in &plan.commands {
        let exit_code = runner
            .run(command)
            .map_err(|source| BootstrapError::InstallIo {
                command: command.display.clone(),
                source,
            })?;
        if exit_code != Some(0) {
            return Err(BootstrapError::InstallFailed {
                command: command.display.clone(),
                exit_code,
            });
        }
    }

    let (requested_path, missing) = inspect_dependency(dependency, requested, runner);
    if missing.is_empty() {
        Ok(EnsureOutcome::Installed(requested_path.expect(
            "standard dependency must contain the requested executable",
        )))
    } else {
        Ok(EnsureOutcome::InstalledButNotVisible { missing })
    }
}

/// Resolve a dependency, offering a confirmed native installation only for the
/// canonical names `ffmpeg`, `pngquant`, `cwebp`, and `git`.
///
/// Explicit paths, custom executable names, and values supplied through a
/// `*_BIN` override are resolve-only and never launch an installer.
pub fn require(tool: &str, requested: &str) -> ToolResult<PathBuf> {
    let policy = request_policy(requested);
    let mut runner = SystemRunner;

    if let RequestPolicy::ResolveOnly(reason) = policy {
        return runner.find(requested).ok_or_else(|| {
            BootstrapError::ResolveOnlyMissing {
                requested: requested.into(),
                reason,
            }
            .into_tool_error(tool)
        });
    }

    let RequestPolicy::Install(dependency) = policy else {
        unreachable!();
    };
    let platform = Platform::current().map_err(|error| error.into_tool_error(tool))?;
    let mut interaction = SystemInteraction::new();
    match ensure_dependency(
        tool,
        requested,
        dependency,
        platform,
        &mut runner,
        &mut interaction,
    )
    .map_err(|error| error.into_tool_error(tool))?
    {
        EnsureOutcome::AlreadyAvailable(path) | EnsureOutcome::Installed(path) => Ok(path),
        EnsureOutcome::Declined => Err(ToolError::cancelled(tool)),
        EnsureOutcome::NeedsInteractiveConfirmation { commands } => Err(ToolError::new(
            tool,
            format!(
                "{} is required but missing; nothing was installed because dependency installation requires interactive confirmation. Run in an interactive terminal:\n{commands}",
                dependency.display_name()
            ),
        )),
        EnsureOutcome::InstalledButNotVisible { missing } => {
            if let Some(path) = known_vendor_install(dependency) {
                return Ok(path);
            }
            Err(ToolError::new(
                tool,
                format!(
                    "installation completed, but {} is not visible yet; open a new terminal and retry",
                    missing.join(", ")
                ),
            ))
        }
    }
}

#[cfg(windows)]
fn known_vendor_install(dependency: Dependency) -> Option<PathBuf> {
    if dependency != Dependency::Pngquant {
        return None;
    }
    directories::BaseDirs::new()
        .map(|base| {
            base.data_local_dir()
                .join("JustTools/deps/pngquant/3.0.3/pngquant/pngquant.exe")
        })
        .filter(|path| path.is_file())
}

#[cfg(not(windows))]
fn known_vendor_install(_dependency: Dependency) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};

    use super::*;

    #[derive(Default)]
    struct FakeRunner {
        found: HashMap<String, PathBuf>,
        calls: Vec<CommandSpec>,
        provides_after_run: Vec<(String, PathBuf)>,
        exit_code: Option<i32>,
    }

    impl FakeRunner {
        fn with(programs: &[&str]) -> Self {
            Self {
                found: programs
                    .iter()
                    .map(|program| ((*program).into(), PathBuf::from(format!("/bin/{program}"))))
                    .collect(),
                exit_code: Some(0),
                ..Self::default()
            }
        }
    }

    impl DependencyRunner for FakeRunner {
        fn find(&mut self, program: &str) -> Option<PathBuf> {
            self.found.get(program).cloned()
        }

        fn run(&mut self, command: &CommandSpec) -> io::Result<Option<i32>> {
            self.calls.push(command.clone());
            if self.exit_code == Some(0) {
                self.found.extend(self.provides_after_run.drain(..));
            }
            Ok(self.exit_code)
        }
    }

    #[derive(Default)]
    struct FakeInteraction {
        interactive: bool,
        answers: VecDeque<bool>,
        prompts: Vec<String>,
        messages: Vec<String>,
    }

    impl DependencyInteraction for FakeInteraction {
        fn is_interactive(&self) -> bool {
            self.interactive
        }

        fn confirm(&mut self, prompt: &str) -> io::Result<bool> {
            self.prompts.push(prompt.into());
            Ok(self.answers.pop_front().unwrap_or(false))
        }

        fn print_line(&mut self, message: &str) {
            self.messages.push(message.into());
        }
    }

    #[test]
    fn standard_names_are_installable_but_paths_and_custom_names_are_not() {
        fn no_environment(_: &str) -> Option<OsString> {
            None
        }
        assert_eq!(
            request_policy_with("ffmpeg", no_environment),
            RequestPolicy::Install(Dependency::Ffmpeg)
        );
        assert!(matches!(
            request_policy_with("tools/ffmpeg", no_environment),
            RequestPolicy::ResolveOnly(_)
        ));
        assert!(matches!(
            request_policy_with("my-ffmpeg", no_environment),
            RequestPolicy::ResolveOnly(_)
        ));
    }

    #[test]
    fn environment_override_is_resolve_only_even_when_value_is_standard_name() {
        let policy = request_policy_with("ffmpeg", |name| {
            (name == "FFMPEG_BIN").then(|| OsString::from("ffmpeg"))
        });
        assert_eq!(
            policy,
            RequestPolicy::ResolveOnly("because FFMPEG_BIN is set".into())
        );
    }

    #[test]
    fn available_ffmpeg_requires_both_ffmpeg_and_ffprobe() {
        let mut runner = FakeRunner::with(&["ffmpeg", "ffprobe"]);
        let mut interaction = FakeInteraction::default();
        let outcome = ensure_dependency(
            "justvideo",
            "ffmpeg",
            Dependency::Ffmpeg,
            Platform::Windows,
            &mut runner,
            &mut interaction,
        )
        .unwrap();
        assert_eq!(
            outcome,
            EnsureOutcome::AlreadyAvailable(PathBuf::from("/bin/ffmpeg"))
        );
        assert!(runner.calls.is_empty());
        assert!(interaction.prompts.is_empty());
    }

    #[test]
    fn non_interactive_missing_dependency_returns_command_without_running() {
        let mut runner = FakeRunner::with(&["winget"]);
        let mut interaction = FakeInteraction::default();
        let outcome = ensure_dependency(
            "justzip",
            "git",
            Dependency::Git,
            Platform::Windows,
            &mut runner,
            &mut interaction,
        )
        .unwrap();
        let EnsureOutcome::NeedsInteractiveConfirmation { commands } = outcome else {
            panic!("unexpected outcome: {outcome:?}");
        };
        assert!(commands.contains("winget install --id Git.Git --exact"));
        assert!(runner.calls.is_empty());
        assert!(interaction.prompts.is_empty());
    }

    #[test]
    fn declined_installation_runs_nothing() {
        let mut runner = FakeRunner::with(&["brew"]);
        let mut interaction = FakeInteraction {
            interactive: true,
            answers: VecDeque::from([false]),
            ..FakeInteraction::default()
        };
        let outcome = ensure_dependency(
            "justwebp",
            "cwebp",
            Dependency::Cwebp,
            Platform::MacOs,
            &mut runner,
            &mut interaction,
        )
        .unwrap();
        assert_eq!(outcome, EnsureOutcome::Declined);
        assert!(runner.calls.is_empty());
        assert_eq!(interaction.prompts.len(), 1);
    }

    #[test]
    fn confirmed_installation_runs_and_rechecks() {
        let mut runner = FakeRunner::with(&["apt-get", "sudo"]);
        runner
            .provides_after_run
            .push(("cwebp".into(), PathBuf::from("/usr/bin/cwebp")));
        let mut interaction = FakeInteraction {
            interactive: true,
            answers: VecDeque::from([true]),
            ..FakeInteraction::default()
        };
        let outcome = ensure_dependency(
            "justwebp",
            "cwebp",
            Dependency::Cwebp,
            Platform::Linux,
            &mut runner,
            &mut interaction,
        )
        .unwrap();
        assert_eq!(
            outcome,
            EnsureOutcome::Installed(PathBuf::from("/usr/bin/cwebp"))
        );
        assert_eq!(runner.calls[0].display, "sudo apt-get install -y webp");
    }

    #[test]
    fn package_manager_mappings_are_exact() {
        let cases = [
            (Platform::Windows, Dependency::Ffmpeg, "Gyan.FFmpeg"),
            (Platform::Windows, Dependency::Cwebp, "Google.Libwebp"),
            (Platform::Windows, Dependency::Git, "Git.Git"),
            (Platform::MacOs, Dependency::Cwebp, "brew install webp"),
            (
                Platform::Linux,
                Dependency::Cwebp,
                "apt-get install -y webp",
            ),
        ];
        for (platform, dependency, expected) in cases {
            let manager = match platform {
                Platform::Windows => "winget",
                Platform::MacOs => "brew",
                Platform::Linux => "apt-get",
            };
            let mut runner = FakeRunner::with(&[manager]);
            let plan = build_install_plan(dependency, platform, &mut runner).unwrap();
            assert!(plan.commands[0].display.contains(expected));
        }
    }

    #[test]
    fn windows_pngquant_fallback_is_checksum_pinned() {
        let mut runner = FakeRunner::with(&["winget", "powershell.exe"]);
        let plan =
            build_install_plan(Dependency::Pngquant, Platform::Windows, &mut runner).unwrap();
        assert_eq!(plan.installer, Installer::VendorArchive);
        assert!(
            plan.commands[0]
                .args
                .last()
                .unwrap()
                .to_string_lossy()
                .contains("BD0257AEECCFE446A4CD764927E26F8AF6051796F28ABED104307284107B120D")
        );
    }

    #[test]
    fn installer_failure_becomes_bootstrap_error() {
        let mut runner = FakeRunner::with(&["brew"]);
        runner.exit_code = Some(7);
        let mut interaction = FakeInteraction {
            interactive: true,
            answers: VecDeque::from([true]),
            ..FakeInteraction::default()
        };
        let error = ensure_dependency(
            "justzip",
            "git",
            Dependency::Git,
            Platform::MacOs,
            &mut runner,
            &mut interaction,
        )
        .unwrap_err();
        assert!(error.to_string().contains("exited with status 7"));
        assert_eq!(runner.calls.len(), 1);
    }
}
