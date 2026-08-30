use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    pub fn current() -> Option<Self> {
        if cfg!(windows) {
            Some(Self::Windows)
        } else if cfg!(target_os = "macos") {
            Some(Self::MacOs)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else {
            None
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::MacOs => "macOS",
            Self::Linux => "Linux",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Section {
    Essentials,
    AiAgents,
    EditorsTerminals,
    RuntimesContainers,
    DataApi,
    Communication,
    Browsers,
    Utilities,
    Creative,
}

impl Section {
    pub fn label(self) -> &'static str {
        match self {
            Self::Essentials => "Essentials",
            Self::AiAgents => "AI & Agents",
            Self::EditorsTerminals => "Editors & Terminals",
            Self::RuntimesContainers => "Runtimes & Containers",
            Self::DataApi => "Data & API",
            Self::Communication => "Communication",
            Self::Browsers => "Browsers",
            Self::Utilities => "Utilities",
            Self::Creative => "Creative",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NativePackages {
    pub apt: &'static [&'static str],
    pub dnf: &'static [&'static str],
    pub pacman: &'static [&'static str],
    pub zypper: &'static [&'static str],
    pub apk: &'static [&'static str],
}

#[derive(Clone, Copy, Debug)]
pub enum Source {
    Winget(&'static str),
    BrewFormula(&'static str),
    BrewCask(&'static str),
    Native(NativePackages),
    Flatpak(&'static str),
    Script {
        url: &'static str,
        shell: &'static str,
        arguments: &'static [&'static str],
    },
}

impl Source {
    pub fn package_key(self) -> Option<&'static str> {
        match self {
            Self::Winget(id) | Self::BrewFormula(id) | Self::BrewCask(id) | Self::Flatpak(id) => {
                Some(id)
            }
            Self::Native(_) | Self::Script { .. } => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Winget(_) => "WinGet",
            Self::BrewFormula(_) | Self::BrewCask(_) => "Homebrew",
            Self::Native(_) => "system package manager",
            Self::Flatpak(_) => "Flathub",
            Self::Script { .. } => "official installer",
        }
    }
}

#[derive(Clone, Debug)]
pub struct App {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub section: Section,
    pub recommended: bool,
    pub probes: &'static [&'static str],
    pub dependencies: &'static [&'static str],
    pub source: Source,
}

struct Definition {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    section: Section,
    recommended: bool,
    probes: &'static [&'static str],
    dependencies: &'static [&'static str],
    windows: Option<Source>,
    macos: Option<Source>,
    linux: Option<Source>,
}

fn native(
    apt: &'static [&'static str],
    dnf: &'static [&'static str],
    pacman: &'static [&'static str],
    zypper: &'static [&'static str],
    apk: &'static [&'static str],
) -> Source {
    Source::Native(NativePackages {
        apt,
        dnf,
        pacman,
        zypper,
        apk,
    })
}

fn definitions() -> Vec<Definition> {
    use Section::{
        AiAgents, Browsers, Communication, Creative, DataApi, EditorsTerminals, Essentials,
        RuntimesContainers, Utilities,
    };
    use Source::{BrewCask, BrewFormula, Flatpak, Script, Winget};

    vec![
        Definition {
            id: "git",
            name: "Git",
            description: "Version control used by modern development workflows",
            section: Essentials,
            recommended: true,
            probes: &["git"],
            dependencies: &[],
            windows: Some(Winget("Git.Git")),
            macos: Some(BrewFormula("git")),
            linux: Some(native(&["git"], &["git"], &["git"], &["git"], &["git"])),
        },
        Definition {
            id: "github-desktop",
            name: "GitHub Desktop",
            description: "Friendly visual Git and GitHub client",
            section: Essentials,
            recommended: true,
            probes: &[],
            dependencies: &[],
            windows: Some(Winget("GitHub.GitHubDesktop")),
            macos: Some(BrewCask("github")),
            linux: None,
        },
        Definition {
            id: "github-cli",
            name: "GitHub CLI",
            description: "Issues, pull requests, releases, and GitHub from the terminal",
            section: Essentials,
            recommended: true,
            probes: &["gh"],
            dependencies: &[],
            windows: Some(Winget("GitHub.cli")),
            macos: Some(BrewFormula("gh")),
            linux: Some(native(
                &["gh"],
                &["gh"],
                &["github-cli"],
                &["github-cli"],
                &["github-cli"],
            )),
        },
        Definition {
            id: "bitwarden",
            name: "Bitwarden",
            description: "Cross-platform password manager",
            section: Essentials,
            recommended: false,
            probes: &[],
            dependencies: &[],
            windows: Some(Winget("Bitwarden.Bitwarden")),
            macos: Some(BrewCask("bitwarden")),
            linux: Some(Flatpak("com.bitwarden.desktop")),
        },
        Definition {
            id: "tailscale",
            name: "Tailscale",
            description: "Simple private networking between your devices",
            section: Essentials,
            recommended: false,
            probes: &["tailscale"],
            dependencies: &[],
            windows: Some(Winget("Tailscale.Tailscale")),
            macos: Some(BrewCask("tailscale-app")),
            linux: None,
        },
        Definition {
            id: "codex",
            name: "Codex CLI",
            description: "OpenAI coding agent for terminal workflows",
            section: AiAgents,
            recommended: true,
            probes: &["codex"],
            dependencies: &[],
            windows: Some(Winget("OpenAI.Codex")),
            macos: Some(BrewCask("codex")),
            linux: Some(Script {
                url: "https://chatgpt.com/codex/install.sh",
                shell: "sh",
                arguments: &[],
            }),
        },
        Definition {
            id: "claude-code",
            name: "Claude Code",
            description: "Anthropic coding agent for terminal workflows",
            section: AiAgents,
            recommended: true,
            probes: &["claude"],
            dependencies: &["git"],
            windows: Some(Winget("Anthropic.ClaudeCode")),
            macos: Some(BrewCask("claude-code")),
            linux: Some(Script {
                url: "https://claude.ai/install.sh",
                shell: "bash",
                arguments: &[],
            }),
        },
        Definition {
            id: "claude-desktop",
            name: "Claude",
            description: "Anthropic desktop app for chat, projects, and artifacts",
            section: AiAgents,
            recommended: true,
            probes: &[],
            dependencies: &[],
            windows: Some(Winget("Anthropic.Claude")),
            macos: Some(BrewCask("claude")),
            linux: None,
        },
        Definition {
            id: "ollama",
            name: "Ollama",
            description: "Run and manage local language models",
            section: AiAgents,
            recommended: false,
            probes: &["ollama"],
            dependencies: &[],
            windows: Some(Winget("Ollama.Ollama")),
            macos: Some(BrewCask("ollama-app")),
            linux: Some(Script {
                url: "https://ollama.com/install.sh",
                shell: "sh",
                arguments: &[],
            }),
        },
        Definition {
            id: "lm-studio",
            name: "LM Studio",
            description: "Desktop app for discovering and running local models",
            section: AiAgents,
            recommended: false,
            probes: &["lms"],
            dependencies: &[],
            windows: Some(Winget("ElementLabs.LMStudio")),
            macos: Some(BrewCask("lm-studio")),
            linux: None,
        },
        Definition {
            id: "zed",
            name: "Zed",
            description: "Fast collaborative code editor",
            section: EditorsTerminals,
            recommended: true,
            probes: &["zed"],
            dependencies: &[],
            windows: Some(Winget("ZedIndustries.Zed")),
            macos: Some(BrewCask("zed")),
            linux: Some(Script {
                url: "https://zed.dev/install.sh",
                shell: "sh",
                arguments: &[],
            }),
        },
        Definition {
            id: "vscode",
            name: "Visual Studio Code",
            description: "Extensible general-purpose code editor",
            section: EditorsTerminals,
            recommended: false,
            probes: &["code"],
            dependencies: &[],
            windows: Some(Winget("Microsoft.VisualStudioCode")),
            macos: Some(BrewCask("visual-studio-code")),
            linux: Some(Flatpak("com.visualstudio.code")),
        },
        Definition {
            id: "cursor",
            name: "Cursor",
            description: "AI-focused code editor",
            section: EditorsTerminals,
            recommended: false,
            probes: &["cursor"],
            dependencies: &[],
            windows: Some(Winget("Anysphere.Cursor")),
            macos: Some(BrewCask("cursor")),
            linux: None,
        },
        Definition {
            id: "warp",
            name: "Warp",
            description: "Modern terminal with blocks and AI assistance",
            section: EditorsTerminals,
            recommended: false,
            probes: &["warp-terminal", "warp"],
            dependencies: &[],
            windows: Some(Winget("Warp.Warp")),
            macos: Some(BrewCask("warp")),
            linux: None,
        },
        Definition {
            id: "powershell",
            name: "PowerShell 7",
            description: "Modern cross-platform automation shell",
            section: EditorsTerminals,
            recommended: false,
            probes: &["pwsh"],
            dependencies: &[],
            windows: Some(Winget("Microsoft.PowerShell")),
            macos: None,
            linux: None,
        },
        Definition {
            id: "windows-terminal",
            name: "Windows Terminal",
            description: "Tabbed terminal host for Windows shells",
            section: EditorsTerminals,
            recommended: false,
            probes: &["wt"],
            dependencies: &[],
            windows: Some(Winget("Microsoft.WindowsTerminal")),
            macos: None,
            linux: None,
        },
        Definition {
            id: "node",
            name: "Node.js LTS",
            description: "Long-term-support JavaScript runtime and npm",
            section: RuntimesContainers,
            recommended: false,
            probes: &["node", "npm"],
            dependencies: &[],
            windows: Some(Winget("OpenJS.NodeJS.LTS")),
            macos: Some(BrewFormula("node")),
            linux: Some(native(
                &["nodejs", "npm"],
                &["nodejs", "npm"],
                &["nodejs", "npm"],
                &["nodejs", "npm"],
                &["nodejs", "npm"],
            )),
        },
        Definition {
            id: "python",
            name: "Python",
            description: "Python runtime with its standard package tooling",
            section: RuntimesContainers,
            recommended: false,
            probes: &["python3", "python"],
            dependencies: &[],
            windows: Some(Winget("Python.Python.3.13")),
            macos: Some(BrewFormula("python")),
            linux: Some(native(
                &["python3", "python3-pip"],
                &["python3", "python3-pip"],
                &["python", "python-pip"],
                &["python3", "python3-pip"],
                &["python3", "py3-pip"],
            )),
        },
        Definition {
            id: "dotnet",
            name: ".NET SDK 10",
            description: "Current LTS SDK for .NET, ASP.NET Core, and desktop apps",
            section: RuntimesContainers,
            recommended: true,
            probes: &["dotnet"],
            dependencies: &[],
            windows: Some(Winget("Microsoft.DotNet.SDK.10")),
            macos: Some(BrewCask("dotnet-sdk")),
            linux: Some(Script {
                url: "https://dot.net/v1/dotnet-install.sh",
                shell: "bash",
                arguments: &["--channel", "LTS"],
            }),
        },
        Definition {
            id: "bun",
            name: "Bun",
            description: "Fast JavaScript runtime, package manager, and toolkit",
            section: RuntimesContainers,
            recommended: false,
            probes: &["bun"],
            dependencies: &[],
            windows: Some(Winget("Oven-sh.Bun")),
            macos: Some(BrewFormula("bun")),
            linux: Some(Script {
                url: "https://bun.sh/install",
                shell: "bash",
                arguments: &[],
            }),
        },
        Definition {
            id: "rustup",
            name: "Rustup",
            description: "Rust compiler and toolchain manager",
            section: RuntimesContainers,
            recommended: false,
            probes: &["rustup", "cargo"],
            dependencies: &[],
            windows: Some(Winget("Rustlang.Rustup")),
            macos: Some(BrewFormula("rustup")),
            linux: Some(Script {
                url: "https://sh.rustup.rs",
                shell: "sh",
                arguments: &["-y"],
            }),
        },
        Definition {
            id: "go",
            name: "Go",
            description: "Go compiler and standard development tools",
            section: RuntimesContainers,
            recommended: false,
            probes: &["go"],
            dependencies: &[],
            windows: Some(Winget("GoLang.Go")),
            macos: Some(BrewFormula("go")),
            linux: Some(native(
                &["golang-go"],
                &["golang"],
                &["go"],
                &["go"],
                &["go"],
            )),
        },
        Definition {
            id: "docker",
            name: "Docker",
            description: "Containers and local development environments",
            section: RuntimesContainers,
            recommended: false,
            probes: &["docker"],
            dependencies: &[],
            windows: Some(Winget("Docker.DockerDesktop")),
            macos: Some(BrewCask("docker-desktop")),
            linux: Some(native(
                &["docker.io"],
                &["moby-engine"],
                &["docker"],
                &["docker"],
                &["docker"],
            )),
        },
        Definition {
            id: "dbeaver",
            name: "DBeaver Community",
            description: "Universal SQL client and database manager",
            section: DataApi,
            recommended: true,
            probes: &["dbeaver"],
            dependencies: &[],
            windows: Some(Winget("DBeaver.DBeaver.Community")),
            macos: Some(BrewCask("dbeaver-community")),
            linux: Some(Flatpak("io.dbeaver.DBeaverCommunity")),
        },
        Definition {
            id: "postman",
            name: "Postman",
            description: "API design, testing, and collaboration",
            section: DataApi,
            recommended: false,
            probes: &[],
            dependencies: &[],
            windows: Some(Winget("Postman.Postman")),
            macos: Some(BrewCask("postman")),
            linux: Some(Flatpak("com.getpostman.Postman")),
        },
        Definition {
            id: "bruno",
            name: "Bruno",
            description: "Git-friendly local API client",
            section: DataApi,
            recommended: false,
            probes: &["bruno"],
            dependencies: &[],
            windows: Some(Winget("Bruno.Bruno")),
            macos: Some(BrewCask("bruno")),
            linux: Some(Flatpak("com.usebruno.Bruno")),
        },
        Definition {
            id: "insomnia",
            name: "Insomnia",
            description: "API client for REST, GraphQL, and gRPC",
            section: DataApi,
            recommended: false,
            probes: &["insomnia"],
            dependencies: &[],
            windows: Some(Winget("Insomnia.Insomnia")),
            macos: Some(BrewCask("insomnia")),
            linux: Some(Flatpak("rest.insomnia.Insomnia")),
        },
        Definition {
            id: "beekeeper",
            name: "Beekeeper Studio",
            description: "Approachable SQL editor and database client",
            section: DataApi,
            recommended: false,
            probes: &[],
            dependencies: &[],
            windows: Some(Winget("beekeeper-studio.beekeeper-studio")),
            macos: Some(BrewCask("beekeeper-studio")),
            linux: Some(Flatpak("io.beekeeperstudio.Studio")),
        },
        Definition {
            id: "telegram",
            name: "Telegram",
            description: "Fast cross-platform messaging desktop client",
            section: Communication,
            recommended: true,
            probes: &["telegram-desktop"],
            dependencies: &[],
            windows: Some(Winget("Telegram.TelegramDesktop")),
            macos: Some(BrewCask("telegram")),
            linux: Some(Flatpak("org.telegram.desktop")),
        },
        Definition {
            id: "notion",
            name: "Notion",
            description: "Connected notes, documents, projects, and knowledge",
            section: Communication,
            recommended: true,
            probes: &[],
            dependencies: &[],
            windows: Some(Winget("Notion.Notion")),
            macos: Some(BrewCask("notion")),
            linux: None,
        },
        Definition {
            id: "discord",
            name: "Discord",
            description: "Communities, chat, calls, and screen sharing",
            section: Communication,
            recommended: false,
            probes: &["discord"],
            dependencies: &[],
            windows: Some(Winget("Discord.Discord")),
            macos: Some(BrewCask("discord")),
            linux: Some(Flatpak("com.discordapp.Discord")),
        },
        Definition {
            id: "slack",
            name: "Slack",
            description: "Team messaging and collaboration",
            section: Communication,
            recommended: false,
            probes: &["slack"],
            dependencies: &[],
            windows: Some(Winget("SlackTechnologies.Slack")),
            macos: Some(BrewCask("slack")),
            linux: Some(Flatpak("com.slack.Slack")),
        },
        Definition {
            id: "signal",
            name: "Signal",
            description: "Private end-to-end encrypted messaging",
            section: Communication,
            recommended: false,
            probes: &["signal-desktop"],
            dependencies: &[],
            windows: Some(Winget("OpenWhisperSystems.Signal")),
            macos: Some(BrewCask("signal")),
            linux: Some(Flatpak("org.signal.Signal")),
        },
        Definition {
            id: "zoom",
            name: "Zoom Workplace",
            description: "Video meetings and screen sharing",
            section: Communication,
            recommended: false,
            probes: &["zoom"],
            dependencies: &[],
            windows: Some(Winget("Zoom.Zoom")),
            macos: Some(BrewCask("zoom")),
            linux: Some(Flatpak("us.zoom.Zoom")),
        },
        Definition {
            id: "firefox",
            name: "Firefox",
            description: "Independent privacy-focused web browser",
            section: Browsers,
            recommended: false,
            probes: &["firefox"],
            dependencies: &[],
            windows: Some(Winget("Mozilla.Firefox")),
            macos: Some(BrewCask("firefox")),
            linux: Some(Flatpak("org.mozilla.firefox")),
        },
        Definition {
            id: "chrome",
            name: "Google Chrome",
            description: "Widely compatible Chromium browser",
            section: Browsers,
            recommended: false,
            probes: &["google-chrome", "chrome"],
            dependencies: &[],
            windows: Some(Winget("Google.Chrome")),
            macos: Some(BrewCask("google-chrome")),
            linux: Some(Flatpak("com.google.Chrome")),
        },
        Definition {
            id: "brave",
            name: "Brave",
            description: "Privacy-oriented Chromium browser",
            section: Browsers,
            recommended: false,
            probes: &["brave-browser", "brave"],
            dependencies: &[],
            windows: Some(Winget("Brave.Brave")),
            macos: Some(BrewCask("brave-browser")),
            linux: Some(Flatpak("com.brave.Browser")),
        },
        Definition {
            id: "7zip",
            name: "7-Zip",
            description: "Small, reliable file archiver",
            section: Utilities,
            recommended: false,
            probes: &["7z", "7zz"],
            dependencies: &[],
            windows: Some(Winget("7zip.7zip")),
            macos: Some(BrewFormula("sevenzip")),
            linux: Some(native(
                &["p7zip-full"],
                &["7zip"],
                &["7zip"],
                &["7zip"],
                &["7zip"],
            )),
        },
        Definition {
            id: "sharex",
            name: "ShareX",
            description: "Powerful screenshots, recording, and upload workflows",
            section: Utilities,
            recommended: true,
            probes: &[],
            dependencies: &[],
            windows: Some(Winget("ShareX.ShareX")),
            macos: None,
            linux: None,
        },
        Definition {
            id: "powertoys",
            name: "PowerToys",
            description: "Microsoft productivity utilities for Windows",
            section: Utilities,
            recommended: false,
            probes: &[],
            dependencies: &[],
            windows: Some(Winget("Microsoft.PowerToys")),
            macos: None,
            linux: None,
        },
        Definition {
            id: "windhawk",
            name: "Windhawk",
            description: "Mod manager for customizing Windows programs and the shell",
            section: Utilities,
            recommended: true,
            probes: &[],
            dependencies: &[],
            windows: Some(Winget("RamenSoftware.Windhawk")),
            macos: None,
            linux: None,
        },
        Definition {
            id: "everything",
            name: "Everything",
            description: "Instant filename search for Windows",
            section: Utilities,
            recommended: true,
            probes: &["es"],
            dependencies: &[],
            windows: Some(Winget("voidtools.Everything")),
            macos: None,
            linux: None,
        },
        Definition {
            id: "winmerge",
            name: "WinMerge",
            description: "Visual file and folder comparison",
            section: Utilities,
            recommended: false,
            probes: &[],
            dependencies: &[],
            windows: Some(Winget("WinMerge.WinMerge")),
            macos: None,
            linux: None,
        },
        Definition {
            id: "keepassxc",
            name: "KeePassXC",
            description: "Offline open-source password manager",
            section: Utilities,
            recommended: false,
            probes: &["keepassxc"],
            dependencies: &[],
            windows: Some(Winget("KeePassXCTeam.KeePassXC")),
            macos: Some(BrewCask("keepassxc")),
            linux: Some(Flatpak("org.keepassxc.KeePassXC")),
        },
        Definition {
            id: "vlc",
            name: "VLC",
            description: "Versatile local media player",
            section: Utilities,
            recommended: false,
            probes: &["vlc"],
            dependencies: &[],
            windows: Some(Winget("VideoLAN.VLC")),
            macos: Some(BrewCask("vlc")),
            linux: Some(Flatpak("org.videolan.VLC")),
        },
        Definition {
            id: "obs",
            name: "OBS Studio",
            description: "Screen recording and live streaming studio",
            section: Creative,
            recommended: false,
            probes: &["obs", "obs-studio"],
            dependencies: &[],
            windows: Some(Winget("OBSProject.OBSStudio")),
            macos: Some(BrewCask("obs")),
            linux: Some(Flatpak("com.obsproject.Studio")),
        },
        Definition {
            id: "figma",
            name: "Figma",
            description: "Collaborative interface design desktop app",
            section: Creative,
            recommended: false,
            probes: &[],
            dependencies: &[],
            windows: Some(Winget("Figma.Figma")),
            macos: Some(BrewCask("figma")),
            linux: None,
        },
        Definition {
            id: "gimp",
            name: "GIMP",
            description: "Open-source image editor",
            section: Creative,
            recommended: false,
            probes: &["gimp"],
            dependencies: &[],
            windows: Some(Winget("GIMP.GIMP.3")),
            macos: Some(BrewCask("gimp")),
            linux: Some(Flatpak("org.gimp.GIMP")),
        },
        Definition {
            id: "inkscape",
            name: "Inkscape",
            description: "Open-source vector graphics editor",
            section: Creative,
            recommended: false,
            probes: &["inkscape"],
            dependencies: &[],
            windows: Some(Winget("Inkscape.Inkscape")),
            macos: Some(BrewCask("inkscape")),
            linux: Some(Flatpak("org.inkscape.Inkscape")),
        },
    ]
}

pub fn for_platform(platform: Platform) -> Vec<App> {
    definitions()
        .into_iter()
        .filter_map(|definition| {
            let source = match platform {
                Platform::Windows => definition.windows,
                Platform::MacOs => definition.macos,
                Platform::Linux => definition.linux,
            }?;
            Some(App {
                id: definition.id,
                name: definition.name,
                description: definition.description,
                section: definition.section,
                recommended: definition.recommended,
                probes: definition.probes,
                dependencies: definition.dependencies,
                source,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_platform_catalog_has_unique_ids() {
        for platform in [Platform::Windows, Platform::MacOs, Platform::Linux] {
            let apps = for_platform(platform);
            let unique = apps.iter().map(|app| app.id).collect::<HashSet<_>>();
            assert_eq!(unique.len(), apps.len(), "duplicates on {platform:?}");
        }
    }

    #[test]
    fn requested_apps_follow_platform_support() {
        let windows = for_platform(Platform::Windows);
        for id in [
            "github-desktop",
            "github-cli",
            "git",
            "sharex",
            "codex",
            "claude-code",
            "claude-desktop",
            "zed",
            "dotnet",
            "telegram",
            "notion",
            "dbeaver",
            "windhawk",
            "everything",
        ] {
            assert!(windows.iter().any(|app| app.id == id), "missing {id}");
        }
        let macos = for_platform(Platform::MacOs);
        for id in ["claude-desktop", "dotnet", "notion"] {
            assert!(macos.iter().any(|app| app.id == id), "missing {id}");
        }
        assert!(!macos.iter().any(|app| app.id == "windhawk"));
        assert!(!macos.iter().any(|app| app.id == "everything"));

        let linux = for_platform(Platform::Linux);
        assert!(!linux.iter().any(|app| app.id == "sharex"));
        assert!(!linux.iter().any(|app| app.id == "github-desktop"));
        assert!(!linux.iter().any(|app| app.id == "claude-desktop"));
        assert!(!linux.iter().any(|app| app.id == "notion"));
        assert!(!linux.iter().any(|app| app.id == "windhawk"));
        assert!(!linux.iter().any(|app| app.id == "everything"));
        assert!(linux.iter().any(|app| app.id == "codex"));
        assert!(linux.iter().any(|app| app.id == "dotnet"));
    }

    #[test]
    fn dependencies_exist_everywhere_the_dependent_app_exists() {
        for platform in [Platform::Windows, Platform::MacOs, Platform::Linux] {
            let apps = for_platform(platform);
            for app in &apps {
                for dependency in app.dependencies {
                    assert!(
                        apps.iter().any(|candidate| candidate.id == *dependency),
                        "{} depends on unavailable {} on {platform:?}",
                        app.id,
                        dependency
                    );
                }
            }
        }
    }
}
