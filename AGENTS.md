# AGENTS.md

## Project overview

`molenest` is a cross-platform command-line tool written in Rust.
It lets users define SSH port-forwarding presets in advance, select one at runtime, and start SSH port forwarding in the background.

Primary use cases include quickly forwarding ports for remote `marimo`, Jupyter Notebook, JupyterLab, VS Code tunnels, web dashboards, or other services without repeatedly typing long SSH commands.

Example scenario: the user registers forwarding presets for local ports `8888` through `88899`, then chooses the desired preset interactively when launching `molenest`.

## Core goals

- Build a reliable Rust CLI tool that works on Windows, macOS, and Linux.
- Allow users to store reusable SSH port-forwarding presets.
- Provide an interactive selector for choosing a preset at runtime.
- Start SSH port forwarding as a background process.
- Provide commands to list, add, edit, remove, start, stop, and inspect forwarding sessions.
- Prefer simple, maintainable Rust code with strong error handling and clear user messages.

## Non-goals for the first version

- Do not implement a custom SSH client unless absolutely necessary.
- Do not store SSH passwords.
- Do not require a daemon/service for the initial MVP.
- Do not implement cloud sync or remote configuration.
- Do not depend on shell-specific behavior where a Rust-native approach is possible.

## Target platforms

The tool must support:

- Windows 10/11
- macOS
- Linux

Platform-specific behavior must be isolated behind small modules when needed.

Important cross-platform considerations:

- Use `std::process::Command` rather than shelling out through `sh`, `bash`, `cmd`, or PowerShell unless there is a strong reason.
- Assume the system has an `ssh` executable available in `PATH`, or allow the user to configure the SSH binary path.
- Use platform-appropriate config directories through the `directories` crate.
- Use platform-safe file locking or session metadata when tracking background processes.
- Avoid Unix-only process assumptions unless protected with `cfg(unix)`.
- Avoid Windows-only assumptions unless protected with `cfg(windows)`.

## Suggested Rust stack

Use these crates unless there is a good reason not to:

- `clap` for CLI argument parsing.
- `serde` and `serde_json` or `toml` for configuration.
- `anyhow` for application-level errors.
- `thiserror` for library/domain errors if useful.
- `inquire` or `dialoguer` for interactive selection.
- `tracing` and `tracing-subscriber` for logging.
- `which` to locate the SSH executable.
- `uuid` for session identifiers if needed.
- `time` or `chrono` for timestamps if needed.

Prefer minimal dependencies for the MVP.

## Configuration design

Use a human-editable config file.
TOML is preferred for readability.

Default config path should be platform-native, for example:

- Linux: `$XDG_CONFIG_HOME/molenest/config.toml` or `~/.config/molenest/config.toml`
- macOS: `$XDG_CONFIG_HOME/molenest/config.toml` or `~/.config/molenest/config.toml`
- Windows: `%XDG_CONFIG_HOME%\\molenest\\config.toml` or `%USERPROFILE%\\.config\\molenest\\config.toml`

Example config:

```toml
[defaults]
ssh_binary = "ssh"

[[forwards]]
name = "jupyter-8888"
host = "my-server"
local_port = 8888
remote_host = "127.0.0.1"
remote_port = 8888
user = "alice"
identity_file = "~/.ssh/id_ed25519"

[[forwards]]
name = "marimo-2718"
host = "gpu-server"
local_port = 2718
remote_host = "127.0.0.1"
remote_port = 2718
user = "alice"
```

The resulting SSH command should be equivalent to:

```text
ssh -N -L <local_port>:<remote_host>:<remote_port> <user>@<host>
```

When `identity_file` is set, add:

```text
-i <identity_file>
```

Optional future fields:

```toml
extra_args = ["-J", "jump-host"]
strict_host_key_checking = true
bind_address = "127.0.0.1"
```

If `bind_address` is provided, construct forwarding as:

```text
<bind_address>:<local_port>:<remote_host>:<remote_port>
```

## CLI design

Use this command structure as the starting point:

```text
molenest
molenest start [NAME]
molenest stop [SESSION_OR_NAME]
molenest list
molenest sessions
molenest add
molenest remove NAME
molenest config path
molenest config edit
molenest doctor
```

Expected behavior:

- `molenest` with no arguments opens an interactive selector and starts the selected forwarding preset.
- `molenest start NAME` starts a named preset directly.
- `molenest list` lists configured forwarding presets.
- `molenest sessions` lists currently known background sessions.
- `molenest stop SESSION_OR_NAME` stops a running forwarding session.
- `molenest add` interactively creates a new forwarding preset.
- `molenest remove NAME` removes a configured preset.
- `molenest config path` prints the config file path.
- `molenest config edit` opens the config file using `$EDITOR`, `%EDITOR%`, or a sensible fallback.
- `molenest doctor` checks config validity, SSH availability, and basic platform compatibility.

## Background process behavior

For the MVP, start the system `ssh` command as a detached/background process and persist session metadata.

Session metadata should include:

- session id
- preset name
- process id when available
- local port
- remote host
- remote port
- SSH host
- start timestamp
- command summary, excluding sensitive values

Store session metadata in a platform-native data directory, for example:

- Linux: `$XDG_DATA_HOME/molenest/sessions/` or `~/.local/share/molenest/sessions/`
- macOS: `$XDG_DATA_HOME/molenest/sessions/` or `~/.local/share/molenest/sessions/`
- Windows:`%XDG_DATA_HOME%\\molenest\\sessions\\` or `%USERPROFILE%\\.local\\share\\molenest\\sessions\\`

Do not assume a process ID is always enough to identify a live SSH session.
On each `sessions` call, validate whether the process still appears to be running where possible.
Clean up stale session files when detected.

Implementation notes:

- On Unix, consider `CommandExt` and process group/session behavior only inside `cfg(unix)` modules.
- On Windows, use Windows-specific process creation flags only inside `cfg(windows)` modules.
- Keep the public process-spawning interface platform-neutral.
- Do not block the CLI after a session is started successfully.

## SSH command construction rules

Construct commands as argument arrays, never by concatenating shell strings.

Good:

```rust
Command::new(ssh_binary)
    .arg("-N")
    .arg("-L")
    .arg(format!("{}:{}:{}", local_port, remote_host, remote_port))
    .arg(destination);
```

Avoid:

```rust
Command::new("sh")
    .arg("-c")
    .arg(format!("ssh -N -L {}:{}:{} {}", local_port, remote_host, remote_port, destination));
```

Security requirements:

- Never log private key contents.
- Never store passwords.
- Do not execute arbitrary shell fragments from config.
- Treat `extra_args` as advanced user-provided SSH arguments, passed as individual arguments only.
- Validate port numbers are in `1..=65535`.
- Validate preset names are non-empty and suitable for display.

## Error handling and UX

User-facing errors should be specific and actionable.

Examples:

- Config file not found: explain how to create one or run `molenest add`.
- SSH executable not found: suggest installing OpenSSH or setting `ssh_binary`.
- Port already in use: say which local port appears unavailable.
- Unknown preset: show nearby configured preset names if possible.
- Failed to start SSH: display the exit/status information when available.

Avoid panics in normal user-facing code paths.
Reserve `unwrap` and `expect` for tests or truly impossible states.

## Suggested project structure

```text
molenest/
├── Cargo.toml
├── AGENTS.md
├── README.md
├── src/
│   ├── main.rs
│   ├── cli.rs
│   ├── config.rs
│   ├── error.rs
│   ├── paths.rs
│   ├── ssh.rs
│   ├── session.rs
│   ├── ui.rs
│   └── platform/
│       ├── mod.rs
│       ├── unix.rs
│       └── windows.rs
└── tests/
    └── config_tests.rs
```

Keep modules small and testable.

## Testing expectations

Add tests for:

- Config parsing.
- Config serialization.
- SSH command argument construction.
- Preset lookup by name.
- Invalid port validation.
- Session metadata read/write.
- Path handling where practical.

Where platform behavior differs, use conditional tests:

```rust
#[cfg(unix)]
#[test]
fn test_unix_behavior() {}

#[cfg(windows)]
#[test]
fn test_windows_behavior() {}
```

Run before committing:

```text
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## MVP implementation order

1. Create Rust project structure.
2. Implement CLI command definitions with `clap`.
3. Implement config file discovery and parsing.
4. Implement forwarding preset model and validation.
5. Implement SSH command argument construction.
6. Implement interactive preset selection.
7. Implement background process spawning.
8. Implement session metadata persistence.
9. Implement `sessions` and stale session cleanup.
10. Implement `stop` for known sessions.
11. Add tests and improve error messages.
12. Write README usage examples.

## README requirements

The README should include:

- What `molenest` does.
- Installation instructions.
- Example config file.
- Common commands.
- Windows/macOS/Linux notes.
- Security notes about SSH keys and passwords.

## Example user flows

Interactive start:

```text
$ molenest
? Select forwarding preset:
> jupyter-8888  my-server  127.0.0.1:8888 -> localhost:8888
  marimo-2718   gpu-server 127.0.0.1:2718 -> localhost:2718

Started jupyter-8888 in the background.
Local URL: http://127.0.0.1:8888
```

Direct start:

```text
$ molenest start jupyter-8888
Started jupyter-8888 in the background.
Local URL: http://127.0.0.1:8888
```

List sessions:

```text
$ molenest sessions
SESSION       PRESET        LOCAL PORT  HOST       STATUS
abc123        jupyter-8888  8888        my-server  running
```

Stop session:

```text
$ molenest stop abc123
Stopped session abc123.
```

## Coding style

- Prefer clear names over clever abstractions.
- Keep functions short when possible.
- Make invalid states hard to represent.
- Keep platform-specific code isolated.
- Use `Result` consistently.
- Write tests for behavior, not implementation details.
- Avoid global mutable state.

## Compatibility notes

The tool should not require administrative privileges.

Port binding may fail if:

- the local port is already in use;
- security software blocks SSH;
- OpenSSH is not installed or not in `PATH`;
- the SSH config requires interactive input;
- the remote host cannot be reached.

The `doctor` command should detect as many of these issues as practical without making destructive changes.

## Future ideas

These are useful but not required for the MVP:

- Port-range generation, such as adding `8888..8899` in one command.
- Optional system tray integration.
- Optional local web UI.
- Optional daemon mode.
- Optional SOCKS proxy presets.
- Optional reverse forwarding support with `-R`.
- Optional jump host helper.
