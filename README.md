# server_tools

Rust terminal UIs (ratatui) for day-to-day server visibility: networking, Docker, and system resources.

## Layout

| Directory | Binary   | Role |
|-----------|----------|------|
| `common/` | (library) | Shared TOML config loading |
| `network/` | `st-network` | Interfaces, throughput, IPv4 TCP listeners |
| `docker/` | `st-docker` | Container list (bollard) |
| `system/` | `st-system` | CPU, memory, swap, disks, top processes |

## Build

```bash
cargo build --release
# binaries: target/release/st-{network,docker,system}
```

## Configuration

Merged TOML (later files override keys, not whole sections): defaults → `/etc/server_tools/config.toml` → `$XDG_CONFIG_HOME/server_tools/config.toml` → `--config` / `SERVER_TOOLS_CONFIG`.

See `config/examples/` for Debian- and Fedora-oriented samples.

Full operational notes (config merge, distros, Docker socket) live in the Memoria wiki page **Topics/ServerTools**.

## Repository

https://github.com/Solrak97/server_tools
