# htop-win

**A native Windows clone of [htop](https://htop.dev/), rebuilt from the ground up in Rust.**

This project aims to provide the beloved Unix `htop` experience on Windows, focusing on performance, minimal binary size (~500KB), and zero-dependency implementation (no heavy frameworks like `ratatui` or `serde`).

## Architecture

The codebase is structured into three main layers:

### 1. Core (`src/`)
*   **`main.rs`**: Entry point. Sets up the terminal, initializes the application, and runs the main event loop.
*   **`app.rs`**: Central state management (`App` struct). Handles business logic like process sorting, tree view construction, and view mode switching.
*   **`input.rs`**: Handles keyboard and mouse events, dispatching them to the appropriate handlers based on the current context.
*   **`terminal.rs`**: **Custom TUI Library**. A minimal implementation replacing `ratatui` (~1700 lines). It handles layouting, double-buffered rendering (diff-based updates), and basic widgets (Block, Table, List).
*   **`json.rs`**: Minimal JSON parser for reading configuration files, replacing `serde_json`.

### 2. System Backend (`src/system/`)
Handles all interactions with the Windows OS.
*   **`mod.rs`**: Aggregates system metrics.
*   **`cpu.rs`**: Queries per-core CPU usage using `NtQuerySystemInformation`.
*   **`memory.rs`**: Retrieves memory and swap usage.
*   **`process.rs`**: Enumerates processes, handles process termination, priority changes, and user lookups.
*   **`cache.rs`**: Caching mechanism to reduce the overhead of frequent Windows API calls.

### 3. UI Frontend (`src/ui/`)
Responsible for rendering the application state.
*   **`mod.rs`**: Main drawing logic.
*   **`header.rs`**: Renders the meters (CPU bars, Memory, Swap, Uptime).
*   **`process_list.rs`**: The main table view showing processes.
*   **`footer.rs`**: The function key bar and status line.
*   **`dialogs.rs`**: Modal dialogs for help, setup, search, and confirmation.

## Development & Implementation Details

### Key Design Decisions
*   **Direct Windows API**: Uses `windows-rs` directly instead of cross-platform wrappers like `sysinfo` to ensure maximum performance and access to Windows-specific features (Efficiency Mode, Elevation status).
*   **Size Optimization**: The release profile is heavily optimized (`opt-level = "z"`, `lto = true`, `strip = true`) to keep the binary under 500KB.
*   **Custom TUI**: To avoid the bloat of standard TUI libraries, a custom minimal solution is used. It employs a double-buffer strategy where only changed cells are written to the terminal.

### Common Patterns
*   **Dialogs**: When opening a modal (like Kill or Info), capture the target process data immediately to prevent the background refresh from invalidating the context.
*   **Input Handling**: Strictly filter for `KeyEventKind::Press` to avoid duplicate actions from key releases.
*   **Windows API**: Combine multiple data requests into single API calls where possible (e.g., getting priority and CPU times together).

## Building and Running

### Prerequisites
*   Rust 1.85+ (2024 edition)
*   Windows 10/11 environment (for running)

### Commands

| Action | Command | Description |
| :--- | :--- | :--- |
| **Build (Dev)** | `cargo build` | Standard debug build. |
| **Build (Release)** | `cargo build --release` | Optimized build for distribution. |
| **Run** | `cargo run --release` | Build and run locally. |
| **Test** | `cargo test` | Run unit and integration tests. |
| **Lint** | `cargo clippy --release` | Run linter checks. |

## Release Process

Releases are automated via GitHub Actions:
1.  Bump version in `Cargo.toml`.
2.  Tag the commit: `git tag -a vX.Y.Z -m "Release notes"`.
3.  Push the tag: `git push origin vX.Y.Z`.
4.  CI builds `amd64` and `arm64` binaries and drafts a release.

## Auto-Update Mechanism
*   The app checks for updates in the background (via GitHub API).
*   Updates are downloaded to `%TEMP%`.
*   On the next application start, the old binary is renamed, and the new one is swapped in before execution begins.
