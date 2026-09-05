# summoning-circle
Summon your processes and ensure they can never die

## Configuration

`summoning-circle` reads a TOML config file listing the processes it should manage.

By default it looks for `~/.summoning-circle/config.toml`. Pass `--config <PATH>` to use a
different file instead. The `~/.summoning-circle` directory also holds the SQLite database that
tracks running process state.

Each process is declared as a `[[process]]` entry, tagged by `type`. The only type today is
`shell`, which launches a command via the shell and keeps it alive:

```toml
[[process]]
name = "api"
type = "shell"
command = "cargo run --release"
cwd = "/Users/me/src/api"          # optional, defaults to the user's home directory
env = { RUST_LOG = "info" }         # optional

[[process]]
name = "tunnel"
type = "shell"
command = "ssh -N -L 5432:localhost:5432 db-host"
```

| Field     | Required | Description                                    |
| --------- | -------- | ------------------------------------------------ |
| `name`    | yes      | Unique identifier for the process                |
| `type`    | yes      | Process kind; only `shell` is supported today    |
| `command` | yes      | Shell command used to launch the process         |
| `cwd`     | no       | Working directory for the command                |
| `env`     | no       | Extra environment variables for the command      |
