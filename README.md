# summoning-circle
Summon your processes and ensure they can never die

## Installing as a launch agent

On macOS, `summoning-circle install` registers a `com.ncipollo.summoning-circle` launchd user
agent that runs `summoning-circle run` at login and restarts it if it ever exits, so the circle
itself can never die:

```
summoning-circle install
```

This writes `~/Library/LaunchAgents/com.ncipollo.summoning-circle.plist`, pointing at the current
executable's absolute path, and loads it with `launchctl`. If `--config <PATH>` was passed to
`install`, that flag is baked into the agent so it's used on every future launch. Logs go to
`~/.summoning-circle/logs/agent.out.log` and `agent.err.log`. Check status with:

```
launchctl print gui/$(id -u)/com.ncipollo.summoning-circle
```

To stop and remove the agent:

```
summoning-circle uninstall
```

`install` and `uninstall` are macOS-only; on other platforms they exit with an error.

## Configuration

`summoning-circle` reads a TOML config file listing the processes it should manage.

By default it looks for `~/.summoning-circle/config.toml`. Pass `--config <PATH>` to use a
different file instead. The `~/.summoning-circle` directory also holds the SQLite database that
tracks running process state.

Each process is declared as a `[[process]]` entry, tagged by `type`, either `shell` (a command
launched and kept alive by holding its child process) or `daemon` (software with its own
start/stop/status commands, controlled through them instead of OS signals):

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

[[process]]
name = "postgres"
type = "daemon"
start = "pg_ctl start"
stop = "pg_ctl stop"
status = "pg_ctl status"
```

`type = "shell"`:

| Field     | Required | Description                                    |
| --------- | -------- | ------------------------------------------------ |
| `name`    | yes      | Unique identifier for the process                |
| `command` | yes      | Shell command used to launch the process         |
| `cwd`     | no       | Working directory for the command                |
| `env`     | no       | Extra environment variables for the command      |

`type = "daemon"`:

| Field    | Required | Description                                                |
| -------- | -------- | ------------------------------------------------------------ |
| `name`   | yes      | Unique identifier for the process                            |
| `start`  | yes      | Command that launches the daemon                             |
| `stop`   | yes      | Command that shuts the daemon down                           |
| `status` | yes      | Command whose exit code reports liveness: 0 alive, non-zero dead |
