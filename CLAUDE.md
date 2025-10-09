# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Pushel** is a desktop notification reminder application written in Rust. It sends periodic desktop notifications (via `notify-send`) and provides a REST API for ad-hoc notifications. The application is designed for Linux systems with XDG desktop environments.

## Build and Development Commands

```sh
# Build the project
cargo build --release

# Run the application (builds if needed)
cargo run

# Run in release mode
cargo run --release

# Check code without building
cargo check

# Format code
cargo fmt

# Run clippy lints
cargo clippy
```

## Architecture

### Single Binary Application

The entire application is contained in `src/main.rs` with no module separation. Key components:

1. **Configuration System**:
   - Config files stored in `$XDG_CONFIG_HOME/pushel` or `$HOME/.config/pushel`
   - `config.json`: Web server settings, listen address, port, logging format
   - `notifications.json`: Array of periodic notification definitions
   - Auto-creates default configs on first run via `create_default_files()`

2. **Notification Engine**:
   - Each notification spawns a separate thread that loops indefinitely
   - Uses `notify-send` command for Linux desktop notifications
   - Intervals parsed via `parse_interval()` (supports `s`, `m`, `h` suffixes)
   - Supports full `notify-send` options: urgency, expire-time, app-name, icon, category, transient

3. **Motion Detection**:
   - `MotionTracker` struct tracks last user activity via `Instant`
   - Notifications only sent if motion detected within last 15 minutes
   - Currently hardcoded to simulate motion on startup (line 250)
   - Designed for future integration with actual motion detection

4. **Web API**:
   - Built with Warp async web framework
   - Single endpoint: `POST /api/v1/notify`
   - Accepts `AdhocNotification` JSON payload
   - Runs on configurable address/port if `webserver_enabled` is true

5. **Logging**:
   - Uses `tracing` crate with configurable formats (`pretty` or `json`)
   - Initialized based on `log_format` in config.json

### Key Data Structures

- `NotificationConfig`: Configuration for periodic notifications (includes interval)
- `AdhocNotification`: API payload for one-time notifications (no interval)
- `AppConfig`: Application-level settings
- `MotionTracker`: Tracks user activity for conditional notification sending

### Threading Model

- Main thread: Runs async web server (if enabled)
- Spawned threads: One per notification config, each loops with `thread::sleep()`
- Motion tracker cloned to each thread (uses `Instant` which is `Copy`)

## Configuration Locations

- Config directory: `$XDG_CONFIG_HOME/pushel` or `$HOME/.config/pushel`
- Config file: `config.json`
- Notifications file: `notifications.json`

## API Usage

Send ad-hoc notification:
```sh
curl -X POST http://127.0.0.1:3030/api/v1/notify \
  -H "Content-Type: application/json" \
  -d '{"title": "Test", "message": "Hello", "urgency": "normal"}'
```

## Dependencies

- **warp**: Async web framework
- **tokio**: Async runtime (full features)
- **serde/serde_json**: Configuration and API serialization
- **tracing/tracing-subscriber**: Structured logging with JSON/pretty formats

## Release Process

The project uses GitHub Actions for automated releases:
- Triggers on pushes to `main` or `feat**` branches
- Builds release binary
- Auto-generates semantic version tags
- Creates GitHub release with fun generated names (e.g., "blubbering-bamboozlebert")
