# Shell helpers: mkcd, claude_, codex_

On Windows, `just install` automatically installs PowerShell launchers for
`mkcd`, `justmkcd`, `just`, `claude_`, and `codex_` beside the native binary.
Run them directly in PowerShell 5.1 or 7+: **no `init` command, profile edit, or
terminal restart is needed** when the install directory is already on `PATH`.
The launchers also work in `-NoProfile` sessions. Only a newly added `PATH`
directory needs a new terminal. Reinstalling upgrades the launchers together
with the native aliases, with the same backup and rollback behavior.

PowerShell picks the `.ps1` launcher automatically; explicitly running
`mkcd.exe` or `just.exe mkcd` bypasses it and cannot move the calling shell.
The installer preserves unrelated scripts and refuses name collisions.

For macOS/Linux shells or an uninstalled portable binary, load helpers once:

| Shell | Setup |
| --- | --- |
| PowerShell 5.1 / 7+ (Windows, macOS, Linux) | `just init powershell \| Out-String \| Invoke-Expression` |
| Bash (macOS, Linux) | `eval "$(just init bash)"` |
| Zsh (macOS, Linux) | `eval "$(just init zsh)"` |
| Fish (macOS, Linux) | `just init fish \| source` |

Add the matching line to `$PROFILE`, `~/.bashrc`, `~/.zshrc`, or
`~/.config/fish/config.fish` to enable it in future sessions. `just init` only
prints the setup; it never edits your profile. Windows Command Prompt (`cmd.exe`)
does not support this integration; use PowerShell on Windows.

The Windows launchers locate `just.exe` beside themselves, so the installed
folder stays portable. Manually loaded functions call the exact executable that
generated them; load them again after moving it. The native aliases still share
the same portable binary; directory creation requires no additional runtime.

## Create and enter a directory

```sh
mkcd my-folder
mkcd "my project"
mkcd -p projects/new-app
mkcd -- --directory-with-a-leading-dash
justmkcd another-folder
just mkcd another-folder
```

Like `mkdir`, the default creates one directory and fails if it already exists
or its parent is missing. `-p` / `--parents` creates missing parents and accepts
an existing directory. Exactly one destination is allowed; invalid arguments
are rejected before creating directories. Existing files are never replaced.
The current shell changes directory only after success. Creation, argument, or
permission failures leave its location unchanged and return a nonzero status
(`$LASTEXITCODE` in PowerShell). Help, version, and cancelling the UI also leave
the location unchanged.

PowerShell consumes an unquoted `--` when invoking a function. Use
`mkcd '--' --directory-with-a-leading-dash` or `mkcd ./--directory` there.
The same quoting rule applies when forwarding a literal `--` through either
CLI shortcut. See [PowerShell argument parsing](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_parsing#the-end-of-parameters-token).

Bare `mkcd`, `justmkcd`, and `just mkcd` open the standard console UI. Choose a
directory and Run. The directory is never saved; **Create parents** is a saved
default. The footer shows the exact headless command. Explicit arguments or
redirected input/output bypass the UI, as with other JustTools commands.

A child executable cannot change its parent shell's directory. The integration
defines `mkcd`, `justmkcd`, and `just` functions, passes a temporary result file
to the binary, then uses the shell's own directory command. Other `just`
commands pass through normally, and selecting Mkcd in the bare `just` browser
also changes the shell's directory. PowerShell pipelines are forwarded to the
underlying command, including piped input to `just json`. Directory names are passed as literal data;
they are never evaluated as code. A pipeline or subshell can only change its
own directory, so invoke the helper directly to move the interactive shell.

Without integration, the native `mkcd` / `justmkcd` aliases and `just mkcd`
create the directory, print its absolute path, and explain the needed setup.
For scripts that only need creation and an output path, use
`just mkcd --print-path DIRECTORY`; this suppresses the setup notice and skips
the shell directory change even when the integration is loaded.

## Claude and Codex shortcuts

```sh
claude_              # claude --dangerously-skip-permissions
codex_               # codex --yolo
claude_ --resume
codex_ resume
```

These two shell functions forward all additional arguments and preserve the
underlying CLI's exit status. They use your existing `claude` and `codex`
installation and explicitly enable those permission-bypass modes. They do not
install either CLI or change its saved configuration. Like other shell helpers,
they are available automatically in Windows PowerShell and after loading
`just init` in other shells. They do not have a JustTools UI
or a `just claude_` / `just codex_` dispatch command.
