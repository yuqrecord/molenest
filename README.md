# molenest

`molenest` is a cross-platform command-line helper for starting SSH local port
forwarding from reusable presets.

It is designed for workflows such as forwarding remote marimo, Jupyter,
JupyterLab, VS Code tunnel, dashboard, or web app ports without retyping long
`ssh -L` commands.

## Installation

From this repository:

```text
cargo install --path .
```

You need a working `ssh` executable in `PATH`, or set `ssh_binary` in the
configuration file.

## Configuration

Show the config path:

```text
molenest config path
```

Create or edit the TOML config:

```text
molenest config edit
```

When a command needs the config file and it does not exist yet, `molenest` asks
whether to create a default file. If you approve, it writes the file and
continues. If you decline, `molenest` prints that the config file is required
and exits successfully without creating anything.

If the config file exists but has no forwarding presets yet, commands that need
a preset will prompt you to edit the config or add a preset, then exit
successfully.

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

## Commands

```text
molenest                 # select a preset interactively and start it
molenest start NAME      # start a preset
molenest stop ID_OR_NAME # stop a known session
molenest list            # list configured presets
molenest sessions        # list known sessions
molenest add             # interactively add a preset
molenest remove NAME     # remove a preset
molenest config path     # print config path
molenest config edit     # open config in $EDITOR or a fallback editor
molenest doctor          # validate config and SSH availability
```

## Platform Notes

`molenest` supports Windows, macOS, and Linux. It starts the system `ssh`
binary directly with argument arrays rather than shell command strings.

Default paths:

- Config: `$XDG_CONFIG_HOME/molenest/config.toml` or `~/.config/molenest/config.toml`
- Sessions: `$XDG_DATA_HOME/molenest/sessions` or `~/.local/share/molenest/sessions`

On Windows, `%XDG_CONFIG_HOME%` and `%XDG_DATA_HOME%` are honored when set;
otherwise the same `.config` and `.local/share` layout under `%USERPROFILE%` is
used.

## Security Notes

`molenest` does not store SSH passwords or private key contents. Prefer keeping
connection details in OpenSSH config and use `molenest` only for forwarding
presets. Advanced `extra_args` are passed as individual arguments to `ssh`; they
are never executed through a shell.
