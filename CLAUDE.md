# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
cargo build              # Debug build
cargo build --release    # Release build (optimized for size)
cargo run --release      # Build and run
cargo test               # Run all tests
cargo test test_name     # Run specific test
```

## Architecture

htop-win is a Windows htop clone using ratatui for TUI rendering. The codebase follows a clear separation:

### Core Modules

- **`main.rs`** - Entry point, terminal setup, main event loop with tick-based refresh
- **`app.rs`** - Application state (`App` struct), process sorting, tree building, view modes (`ViewMode` enum)
- **`input.rs`** - Keyboard and mouse event handling, dispatches to mode-specific handlers
- **`config.rs`** - Configuration struct (refresh rate, default settings)

### System Module (`system/`)

- **`mod.rs`** - `SystemMetrics` struct aggregating all system data, refresh logic
- **`cpu.rs`** - CPU per-core usage via sysinfo
- **`memory.rs`** - Memory/swap stats, `format_bytes()` helper
- **`process.rs`** - Process enumeration, Windows API calls for:
  - User lookup via process tokens (`get_process_owner`)
  - Priority/nice via `GetPriorityClass`
  - CPU time via `GetProcessTimes`
  - Process termination via `TerminateProcess`

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

## Dependency Version Alignment

Keep versions aligned to avoid duplicate crates:
- `crossterm` version must match what ratatui uses
- `windows` crate version must match what sysinfo uses
