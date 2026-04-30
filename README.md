# molenest

`molenest` is a cross-platform desktop app for starting and monitoring SSH local
port forwarding from reusable presets.

It is designed for workflows such as forwarding remote marimo, Jupyter,
JupyterLab, VS Code tunnel, dashboard, or web app ports without retyping long
`ssh -L` commands.

The app is built with Rust and Slint. It keeps SSH forwarding processes attached
to the running app, so the window can show status, recent SSH output, local URLs,
and failures. Closing the app stops the SSH processes it started during that run.

## Requirements

- Rust 2024 edition capable toolchain.
- A working `ssh` executable in `PATH`, or `ssh_binary` set in the config file.
- Windows 10/11, macOS, or Linux.

## Run From Source

```text
cargo run
```

Build a release binary:

```text
cargo build --release
```

The release executable is written to:

- Windows: `target/release/molenest.exe`
- macOS/Linux: `target/release/molenest`

On Windows release builds, `molenest` uses the Windows GUI subsystem so launching
the `.exe` does not open an extra console window.

## Create A Windows `.exe`

From Windows:

```text
cargo build --release
```

The app executable will be:

```text
target/release/molenest.exe
```

For distribution, ship that `.exe` together with any runtime assets required by
your target environment. The MVP does not yet include an installer or auto-update
flow.

## Create A macOS `.app`

Install `cargo-bundle` once:

```text
cargo install cargo-bundle
```

Create an app bundle:

```text
cargo bundle --release
```

The generated app is typically written to:

```text
target/release/bundle/osx/molenest.app
```

For sharing outside your own machine, macOS signing and notarization are still
needed. Those release steps are not automated in the MVP.

## Configuration

`molenest` uses a human-editable TOML config file.

Default path:

- Linux: `$XDG_CONFIG_HOME/molenest/config.toml` or `~/.config/molenest/config.toml`
- macOS: `$XDG_CONFIG_HOME/molenest/config.toml` or `~/.config/molenest/config.toml`
- Windows: `%XDG_CONFIG_HOME%\\molenest\\config.toml` or `%USERPROFILE%\\.config\\molenest\\config.toml`

If the config file does not exist, the app creates a default empty config on
startup. Fill in **Name**, **Host**, **Local Port**, **Remote Host**, and
**Remote Port**, then press **Add Preset** to append a preset to the config
file. Use **Reload** after editing the TOML file outside the app.

Example:

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

The `host` field is normally an OpenSSH `Host` alias from your `.ssh/config`.
For example, if your SSH config contains `Host my-server`, `molenest` passes
`my-server` directly to `ssh` and lets OpenSSH apply `HostName`, `User`, `Port`,
`IdentityFile`, `ProxyJump`, `ProxyCommand`, and host key settings.

## Main Workflow

1. Open `molenest`.
2. Add a preset with the form if the list is empty.
3. Select a forwarding preset.
4. Press **Start**.
5. Watch the status and recent SSH output in the detail panel.
6. Use the local URL shown by the app.
7. Press **Stop** or close the app to end the SSH process.

The MVP intentionally manages connections only while the GUI app is running.
There is no daemon or detached session store yet.

## Doctor

Use **Doctor** in the app to check:

- config validity;
- SSH executable availability;
- whether each configured local port appears bindable.

The port check is best-effort. A port can still become unavailable between the
doctor check and starting SSH.

## Security Notes

`molenest` does not store SSH passwords or private key contents. Prefer keeping
connection details in OpenSSH config and use `molenest` only for forwarding
presets. Advanced `extra_args` are passed as individual arguments to `ssh`; they
are never executed through a shell.

## Developer Checks

Run the usual checks before committing:

```text
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo doc --no-deps --all-features
```

Use `--no-deps` for documentation checks. Bare `cargo doc` also generates local
documentation for dependency crates, which is unnecessary for this project.

Open the generated API docs locally:

```text
open target/doc/molenest/index.html
```
