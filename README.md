# Anubis

**Anubis** is a lightweight local forward proxy with adaptive routing. It routes outgoing traffic either directly or through one of several user-configured upstreams (HTTP, SOCKS5), based on configurable rules and proxy health.

> **Name:** Anubis
> **Binary:** `anubis`
> **Language:** Rust (Edition 2024, Rust 1.85+)
> **Config format:** HOCON (no YAML)
> **License:** MIT

---

## Why Anubis?

Anubis is designed for users who want a **lightweight ElvUI-style UI** approach to proxy routing: it skins Blizzard systems, keeps a minimal uniform config surface, and avoids knob-creep. You edit the config by hand (HOCON), and Anubis handles the rest.

It's ideal if you want:
- A single binary, no systemd services or daemons required
- CLI-driven operation (start, print config, validate config)
- Adaptive routing with health checks and failover/lowest-latency strategies
- Hot config reload while running

---

## Quick Start

### 1. Build

```bash
cargo build --release
```

The binary will be at `target/release/anubis`.

### 2. Configure

Edit `proxy.conf` (in the project root). See the [Configuration](#configuration) section below.

### 3. Start

```bash
# In the background, watching for config changes
./target/release/anubis --watch

# Or just start (no config flags needed — defaults to proxy.conf)
./target/release/anubis
```

The server binds:

| Port | Protocol | Description |
|---|---|---|
| `8080` | REST API | Health, config, stats |
| `4001` | Forward proxy | HTTP CONNECT proxy listener |

---

## Installation

### Prebuilt binaries

Download the latest release from the [releases page](https://github.com/your-org/anubis/releases).

- **Linux x86_64:** `anubis-linux-x86_64`
- **Linux aarch64:** `anubis-linux-aarch64`
- **macOS (Intel):** `anubis-darwin-x86_64`
- **macOS (Apple Silicon):** `anubis-darwin-aarch64`
- **Windows x86_64:** `anubis-windows-x86_64.exe`

Make the binary executable (`chmod +x anubis`) and run:

```bash
./anubis --version
```

### Build from source

```bash
cargo build --release
```

The binary will be at `target/release/anubis`.

### Configuration

Edit `proxy.conf` (in the project root). See the [Configuration](#configuration) section below.

### Run

```bash
# In the background, watching for config changes
./target/release/anubis --watch

# Or just start (no config flags needed — defaults to proxy.conf)
./target/release/anubis
```

---

## CLI

```bash
anubis [OPTIONS] [COMMAND]
```

Commands:

- `start` — Start the proxy server (default)
- `config` — Configuration inspection
- `help` — Print this message or the help of the given subcommand(s)

Options:

- `-c`, `--config <CONFIG>` — Path to HOCON config file [default: proxy.conf]
- `-l`, `--log-level <LOG_LEVEL>` — Log level (default: info)
- `-w`, `--watch` — Watch config file for changes (hot reload)
- `-h`, `--help` — Print help
- `-V`, `--version` — Print version

### Commands

| Command | Description |
|---|---|
| `anubis start` | Start the proxy server (same as no command) |
| `anubis config print` | Print the resolved, validated configuration to stdout |
| `anubis config validate` | Validate the config file and report any errors |

**Example: print the resolved config**

```bash
anubis config print
```

**Example: validate before starting**

```bash
anubis config validate
```

---

## Configuration (`proxy.conf`)

Anubis uses **HOCON** format. The config file has three sections: `listen`, `proxies`, and `rules`.

### `listen`

```hocon
listen {
  host = "127.0.0.1"
  port = 4001
}
```

| Field | Type | Default | Description |
|---|---|---|---|
| `host` | string | `127.0.0.1` | Listen address for the forward proxy |
| `port` | int | `4001` | Listen port for the forward proxy |

### `proxies`

Each entry in `proxies` is a named upstream. The proxy router selects between these upstreams based on rules and group strategies.

```hocon
proxies = [
  {
    name = "office"
    type = "http"       // or "socks5"
    host = "proxy.company.com"
    port = 8080
    health {
      interval = 30     // seconds between health checks
      timeout = 5       // seconds timeout
    }
  },
  {
    name = "vpn"
    type = "socks5"
    host = "vpn.example.com"
    port = 1080
    health {
      interval = 60
      timeout = 10
    }
  }
]
```

| Field | Type | Description |
|---|---|---|
| `name` | string | Unique identifier for this proxy |
| `type` | `http` \| `socks5` | Transport type |
| `host` | string | Upstream proxy hostname |
| `port` | int | Upstream proxy port |
| `health.interval` | int | Seconds between health checks |
| `health.timeout` | int | Seconds before a health check is considered failed |

**Built-in proxy types:**
- `http` — HTTP CONNECT proxy
- `socks5` — SOCKS5 proxy (RFC 1928)

### `rules`

Rules match outgoing connections and route them to a proxy (or group) or direct.

```hocon
rules = [
  {
    domain = "*.company.com"
    route = "office"
  },
  {
    domain = "*.streaming.com"
    route = "vpn"
  },
  {
    domain = "github.com"
    route = "direct"
  },
  {
    cidr = "10.0.0.0/8"
    route = "direct"
  },
  {
    route = "direct"               // default fallback
  }
]
```

| Field | Type | Description |
|---|---|---|
| `domain` | string | Hostname pattern (*.example.com) |
| `cidr` | string | IP range (e.g. `10.0.0.0/8`) |
| `route` | string | Proxy name, group name, or `direct` |

**Matching order:** Rules are evaluated top to bottom. The first matching rule wins.

---

## How It Works

```
Client → [Proxy] → (selected upstream) → destination
          |
          ├── Rule match (domain/CIDR)
          ├── Group strategy (failover/lowest_latency)
          └── Health check (per-proxy)
```

1. **Client** connects to Anubis (port 4001) and sends a `CONNECT` request.
2. **Proxy listener** receives the connection and routes it.
3. **Route engine** matches the destination against `rules`.
4. **Upstream manager** selects a proxy from the matched group using the group's strategy.
5. **Upstream connector** establishes the connection (Direct, HTTP CONNECT, or SOCKS5).
6. **Health checker** monitors each proxy in the background. If a proxy's last check failed, connections routed to it get an immediate `502` instead of waiting for a timeout (they never fall back to a direct connection). A proxy that has not been checked yet is treated as usable, and it recovers on the next passing check.
7. **REST API** exposes health and stats at `127.0.0.1:8080`.

---

## REST API

| Endpoint | Method | Description |
|---|---|---|
| `/health` | GET | Health status |
| `/proxies` | GET | List of proxies and their health |
| `/stats` | GET | Connection statistics |
| `/reload` | GET/POST/PATCH | Hot reload config (if `--watch` is off) |

---

## Development

### Build

```bash
cargo build
```

### Test

```bash
cargo test
```

### Format

```bash
cargo fmt
```

### Lint

```bash
cargo clippy
```

---

## Continuous Integration

The project uses GitHub Actions for CI/CD:

- **CI**: Runs on every push to `main` and on pull requests. Builds the project and runs all tests on Linux, macOS, and Windows.
- **Release**: Triggered when a version tag (e.g., `v0.1.0`) is pushed to the repository. Builds release binaries for all supported platforms and creates a GitHub release with the artifacts.

See `.github/workflows/ci.yml` and `.github/workflows/release.yml` for the workflow definitions.

---

## License

MIT
