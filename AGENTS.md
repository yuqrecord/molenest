# AGENTS.md

## Project overview

`molenest` is a cross-platform desktop GUI application written in Rust with Slint.
It lets users define SSH port-forwarding presets in advance, select one in a graphical interface, and keep SSH port-forwarding processes visible and controllable while the application window is open.

Primary use cases include quickly forwarding ports for remote `marimo`, Jupyter Notebook, JupyterLab, VS Code tunnels, web dashboards, or other services without repeatedly typing long SSH commands.

Example scenario: the user registers forwarding presets for local ports `8888` through `8899`, then starts and stops the desired preset from the `molenest` window while watching connection status and logs.

`molenest` should rely on the user's existing OpenSSH configuration wherever possible.
Connection details such as host aliases, usernames, ports, identity files, jump hosts, proxy commands, and host key behavior should be inherited from the system `ssh` command and the user's `.ssh/config` rather than duplicated in `molenest` configuration.

## Core goals

- Build a reliable Rust + Slint desktop app that works on Windows, macOS, and Linux.
- Allow users to store reusable SSH port-forwarding presets.
- Provide a clear GUI for listing presets, starting connections, stopping connections, and inspecting connection status.
- Keep SSH forwarding processes attached to and monitored by the running application.
- Surface SSH process state, stderr output, local URLs, and actionable errors in the UI.
- End managed forwarding processes when the application exits, unless a future explicit detached mode is implemented.
- Prefer simple, maintainable Rust code with strong error handling and clear user messages.

## Non-goals for the first version

- Do not implement a custom SSH client unless absolutely necessary.
- Do not store SSH passwords.
- Do not require a daemon/service for the initial MVP.
- Do not keep connections running after the GUI application exits.
- Do not implement system tray or menu bar background residency for the initial MVP.
- Do not implement cloud sync or remote configuration.
- Do not depend on shell-specific behavior where a Rust-native approach is possible.
- Do not reimplement OpenSSH config parsing or duplicate SSH connection profile management.

## Target platforms

The app must support:

- Windows 10/11
- macOS
- Linux

Platform-specific behavior must be isolated behind small modules when needed.

Important cross-platform considerations:

- Use `std::process::Command` or an async process wrapper rather than shelling out through `sh`, `bash`, `cmd`, or PowerShell unless there is a strong reason.
- Assume the system has an `ssh` executable available in `PATH`, or allow the user to configure the SSH binary path.
- Let the system `ssh` executable resolve `~/.ssh/config`, `Host` aliases, `User`, `Port`, `IdentityFile`, `ProxyJump`, `ProxyCommand`, and related OpenSSH settings.
- Use platform-appropriate config directories through the `directories` crate.
- Keep process management platform-neutral at the public API boundary.
- Avoid Unix-only process assumptions unless protected with `cfg(unix)`.
- Avoid Windows-only assumptions unless protected with `cfg(windows)`.
- Build a normal desktop window application: Windows taskbar and macOS Dock support are sufficient for the MVP.

## Suggested Rust stack

Use these crates unless there is a good reason not to:

- `slint` and `slint-build` for the desktop UI.
- `serde` and `toml` for configuration.
- `anyhow` for application-level errors.
- `thiserror` for library/domain errors if useful.
- `tokio` if async process monitoring or non-blocking log streaming is needed.
- `tracing` and `tracing-subscriber` for logging.
- `which` to locate the SSH executable.
- `time` or `chrono` for timestamps if needed.
- `open` or `webbrowser` only if opening local URLs from the UI is implemented.

Avoid adding `clap`, `inquire`, or `dialoguer` for the GUI MVP unless a small compatibility CLI is explicitly needed.
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

[[forwards]]
name = "marimo-2718"
host = "gpu-server"
local_port = 2718
remote_host = "127.0.0.1"
remote_port = 2718
```

The resulting SSH command should be equivalent to:

```text
ssh -N -L <local_port>:<remote_host>:<remote_port> <host>
```

The `host` field should normally be an OpenSSH `Host` alias from the user's `.ssh/config`.
For example, if `.ssh/config` contains:

```text
Host my-server
    HostName example.com
    User alice
    Port 2222
    IdentityFile ~/.ssh/id_ed25519
    ProxyJump jump-host
```

then `host = "my-server"` should be enough for `molenest`.
`molenest` should pass `my-server` as the SSH destination and let `ssh` apply the full OpenSSH configuration.

Optional future fields:

```toml
extra_args = ["-J", "jump-host"]
strict_host_key_checking = true
bind_address = "127.0.0.1"
user = "alice"
identity_file = "~/.ssh/id_ed25519"
```

If `bind_address` is provided, construct forwarding as:

```text
<bind_address>:<local_port>:<remote_host>:<remote_port>
```

`user` and `identity_file` may be considered later as explicit per-preset overrides, but they should not be required for the MVP.
If implemented, they must be translated to normal SSH arguments such as `alice@my-server` or `-i <identity_file>` without bypassing `.ssh/config`.

## GUI design

Use Slint for the primary application UI.

The first screen should be the working connection manager, not a landing page.
Expected UI areas:

- A preset list showing name, SSH host, local port, remote host, remote port, and current status.
- Start and stop controls for the selected preset.
- A connection detail area showing command summary, local URL, process state, start time, and recent SSH output.
- Controls for adding, editing, removing, and reloading presets.
- A configuration path/edit action.
- A doctor/check action for config validity and SSH availability.

Expected behavior:

- Opening `molenest` launches the Slint desktop app.
- Starting a preset spawns `ssh` as a child process owned by the running app.
- The UI updates when the process starts, exits, fails, or writes useful output.
- Stopping a connection terminates the corresponding child process.
- Closing the app stops all managed SSH processes before exit.
- If the app crashes or is forcibly killed, best-effort cleanup is acceptable for the MVP; avoid promising detached process management without a daemon.

UI guidance:

- Keep the app utilitarian and easy to scan.
- Do not build a marketing-style hero page.
- Prefer compact tables/lists, clear status indicators, and explicit controls.
- Avoid decorative UI that distracts from connection state and logs.
- Make error states visible and actionable.

## Process management behavior

For the MVP, do not start SSH as an intentionally detached/background process.
The running GUI application is the process manager.

Runtime connection state should include:

- preset name
- process id when available
- local port
- remote host
- remote port
- SSH host
- start timestamp
- command summary, excluding sensitive values
- current status: idle, starting, running, stopping, stopped, failed, exited
- recent stdout/stderr lines where available
- exit status when available

Implementation notes:

- Construct and spawn `ssh` as a child process.
- Capture stderr so failures such as bad host aliases, authentication errors, or port binding errors can be shown in the UI.
- Do not block the Slint event loop while waiting for SSH.
- Use background tasks, channels, or Slint event-loop invocation APIs to update UI state from process watchers.
- On app shutdown, terminate running children and wait briefly where practical.
- Keep Unix and Windows termination behavior behind small platform-specific modules if needed.

Session metadata files are not required for the GUI MVP because the app should not claim to manage connections after it exits.
If detached sessions are added later, implement them as a separate explicit mode with stronger process supervision semantics.

## SSH command construction rules

Construct commands as argument arrays, never by concatenating shell strings.

Good:

```rust
Command::new(ssh_binary)
    .arg("-N")
    .arg("-L")
    .arg(format!("{}:{}:{}", local_port, remote_host, remote_port))
    .arg(host);
```

Here `host` is the configured SSH destination, usually an OpenSSH `Host` alias.
Do not expand or reinterpret `.ssh/config` inside `molenest`; the spawned `ssh` process should do that.

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
- Prefer `.ssh/config` for SSH connection details. Avoid encouraging users to store usernames, identity files, jump hosts, or proxy commands in `molenest` config unless they are explicit overrides.
- Validate port numbers are in `1..=65535`.
- Validate preset names are non-empty and suitable for display.

## Error handling and UX

User-facing errors should be specific and actionable.

Examples:

- Config file not found: explain how to create one from the UI or open the config path.
- SSH executable not found: suggest installing OpenSSH or setting `ssh_binary`.
- SSH host alias not found or cannot be resolved: suggest checking the `host` value and the user's `.ssh/config`.
- Port already in use: say which local port appears unavailable.
- Unknown preset: refresh the preset list and show the configured preset names.
- Failed to start SSH: display the exit/status information and recent stderr when available.

Avoid panics in normal user-facing code paths.
Reserve `unwrap` and `expect` for tests or truly impossible states.

## Suggested project structure

```text
molenest/
├── Cargo.toml
├── AGENTS.md
├── README.md
├── build.rs
├── src/
│   ├── main.rs
│   ├── app.rs
│   ├── config.rs
│   ├── error.rs
│   ├── paths.rs
│   ├── process.rs
│   ├── ssh.rs
│   ├── ui.rs
│   └── platform/
│       ├── mod.rs
│       ├── unix.rs
│       └── windows.rs
├── ui/
│   └── main.slint
└── tests/
    ├── config_tests.rs
    └── ssh_tests.rs
```

Keep modules small and testable.
Keep Slint markup focused on presentation and callbacks; keep configuration, validation, SSH command construction, and process lifecycle logic in Rust.

## Testing expectations

Add tests for:

- Config parsing.
- Config serialization.
- SSH command argument construction.
- SSH command construction that preserves the configured host alias and does not inline `.ssh/config` details.
- Preset lookup by name.
- Invalid port validation.
- Runtime connection state transitions where practical.
- Process command summary redaction where practical.
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
cargo doc --no-deps --all-features
```

Generate only `molenest` documentation. Do not run bare `cargo doc` for project
verification because Cargo's default behavior also documents dependency crates.

When UI changes are substantial, run the app locally and visually inspect the main states on at least one desktop platform.

## MVP implementation order

1. Create or update the Rust project structure for Slint.
2. Add Slint build integration and a minimal application window.
3. Implement config file discovery and parsing.
4. Implement forwarding preset model and validation.
5. Implement SSH command argument construction.
6. Render the preset list in the Slint UI.
7. Implement start/stop process management for a selected preset.
8. Stream process status and recent stderr/stdout into the UI.
9. Stop all managed processes on application shutdown.
10. Implement add/edit/remove/reload config flows.
11. Implement doctor checks for config validity and SSH availability.
12. Add tests and improve error messages.
13. Write README usage examples and packaging notes.

## README requirements

The README should include:

- What `molenest` does.
- Installation and build instructions.
- How to run the desktop app.
- Example config file.
- How `host` maps to an existing OpenSSH `.ssh/config` `Host` alias.
- Main UI workflows.
- Windows/macOS/Linux notes.
- Packaging notes for Windows `.exe` and macOS `.app` where practical.
- Security notes about SSH keys and passwords.
- Clarification that MVP connections are managed only while the app is running.

## Example user flows

Start a connection:

```text
Open molenest.
Select "jupyter-8888".
Press Start.

Status changes from idle -> starting -> running.
Local URL: http://127.0.0.1:8888
```

Inspect a failure:

```text
Select a failed connection.
Review recent SSH output in the detail panel.
Fix the host alias, SSH config, or local port conflict.
Press Start again.
```

Stop a connection:

```text
Select a running connection.
Press Stop.

Status changes from running -> stopping -> stopped.
```

Exit the app:

```text
Close the molenest window.
The app stops all SSH processes it started during this run.
```

## Coding style

- Prefer clear names over clever abstractions.
- Keep functions short when possible.
- Make invalid states hard to represent.
- Keep platform-specific code isolated.
- Use `Result` consistently.
- Write tests for behavior, not implementation details.
- Avoid global mutable state.
- Keep UI state models explicit and easy to reason about.

## Compatibility notes

The app should not require administrative privileges.

Port binding may fail if:

- the local port is already in use;
- security software blocks SSH;
- OpenSSH is not installed or not in `PATH`;
- the SSH config requires interactive input;
- the `host` alias is missing or misconfigured in `.ssh/config`;
- the remote host cannot be reached.

The doctor action should detect as many of these issues as practical without making destructive changes.

## Packaging notes

Rust can build normal executable artifacts for supported targets.

- Windows: build a `.exe`; use the Windows subsystem for release GUI builds to avoid an unwanted console window.
- macOS: build the Rust executable and bundle it as a `.app` using a packaging tool or project script.
- Linux: build a native executable; future packaging may use AppImage, deb, rpm, or distribution-specific packages.

Signing, notarization, installer generation, and auto-update are outside the MVP unless explicitly requested.

## Future ideas

These are useful but not required for the MVP:

- Port-range generation, such as adding `8888..8899` in one action.
- Optional system tray integration.
- Optional macOS menu bar status item integration.
- Optional local web UI.
- Optional daemon or detached mode for connections that survive app exit.
- Optional SOCKS proxy presets.
- Optional reverse forwarding support with `-R`.
- Optional jump host helper.
