# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
cargo build              # Debug build
cargo build --release    # Release build (optimized for size)
cargo run --release      # Build and run
cargo test               # Run all tests
cargo test test_name     # Run specific test
cargo clippy --release   # Run linter
```

## Architecture

htop-win is a Windows htop clone with a custom TUI library. The codebase follows a clear separation:

### Core Modules

- **`main.rs`** - Entry point, terminal setup, main event loop with tick-based refresh
- **`app.rs`** - Application state (`App` struct), process sorting, tree building, view modes (`ViewMode` enum)
- **`input.rs`** - Keyboard and mouse event handling, dispatches to mode-specific handlers
- **`config.rs`** - Configuration struct (refresh rate, default settings)
- **`terminal.rs`** - Custom minimal TUI library replacing ratatui (~1700 lines). Provides: Layout, Buffer, Terminal, Frame, and widgets (Block, Paragraph, Table, List, Scrollbar)
- **`json.rs`** - Minimal JSON parser for config files (replaces serde_json)

### System Module (`system/`)

- **`mod.rs`** - `SystemMetrics` struct aggregating all system data, refresh logic
- **`cpu.rs`** - CPU per-core usage via direct Windows API (`NtQuerySystemInformation`)
- **`memory.rs`** - Memory/swap stats via Windows API, `format_bytes()` helper
- **`cache.rs`** - Process data caching to reduce Windows API calls
- **`process.rs`** - Process enumeration via `NtQuerySystemInformation`, Windows API calls for:
  - User lookup via process tokens (`get_process_owner`)
  - Priority/nice via `GetPriorityClass`
  - CPU time via `GetProcessTimes`
  - Process termination via `TerminateProcess`
  - Architecture detection via `IsWow64Process2`

### UI Module (`ui/`)

- **`mod.rs`** - Main `draw()` function, layout calculation
- **`header.rs`** - CPU bars, memory bars, tasks/uptime display
- **`process_list.rs`** - Process table with sorting, selection, colors
- **`footer.rs`** - Function key bar, status line
- **`dialogs.rs`** - Modal dialogs (help, search, filter, kill confirm, process info)

## Key Patterns

### Dialog Race Condition Prevention
When opening dialogs (Kill, ProcessInfo), capture the target process immediately to prevent background refresh from changing what's displayed:
```rust
pub fn enter_kill_mode(&mut self) {
    if let Some(proc) = self.selected_process() {
        self.kill_target = Some((proc.pid, proc.name.clone(), proc.command.clone()));
        self.view_mode = ViewMode::Kill;
    }
}
```

### Key Event Filtering
Only handle `KeyEventKind::Press` to prevent "key bounce" issues:
```rust
if key.kind != KeyEventKind::Press {
    return false;
}
```

### Windows API Optimization
Process info collection combines multiple Windows API calls into single `OpenProcess`:
```rust
fn get_win_process_info(pid: u32) -> WinProcessInfo  // Gets priority + CPU time in one call
```

### Custom Terminal Library (terminal.rs)
The custom TUI library uses double-buffered rendering with diff-based updates:
- `Buffer` holds cells with symbol, fg, bg, modifiers
- `Terminal.draw()` compares current vs previous buffer, only updates changed cells
- `Layout.split()` handles `Constraint::Min` as flexible (expands to fill space)
- Widget styles: Apply background first, then render content to preserve span colors

## Dependencies

Minimal dependency set for small binary size:
- `crossterm` - Terminal events and manipulation
- `windows` - Direct Windows API bindings (no sysinfo wrapper)
- `bitflags` - Modifier flags for terminal styling
- `unicode-width` - Character width calculation
- `lexopt` - Lightweight argument parsing

## Releases & Auto-Update

### Creating a Release
1. Update version in `Cargo.toml`
2. Commit and push changes
3. Create annotated tag: `git tag -a v0.0.X -m "Release notes here"`
4. Push tag: `git push origin v0.0.X`
5. GitHub Actions (`.github/workflows/release.yml`) automatically:
   - Builds for x86_64 (amd64) and aarch64 (arm64)
   - Creates release with `htop-win-amd64.exe` and `htop-win-arm64.exe`

### Auto-Update Flow (`installer.rs`)
1. **Background check**: 3 seconds after startup, spawns thread to check GitHub API
2. **Architecture detection**: Selects correct binary (amd64/arm64) based on `cfg!(target_arch)`
3. **Download**: Downloads to `%TEMP%\htop-win-update.exe`
4. **Notification**: Shows "Update vX.Y.Z downloaded. Restart to apply." in status bar
5. **Apply on restart**: Before UI starts, `apply_pending_update()`:
   - Renames running `htop.exe` → `htop.exe.old` (Windows allows renaming running exe)
   - Copies update → `htop.exe`
   - Cleans up temp and backup files
