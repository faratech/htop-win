# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
cargo build              # Debug build
cargo build --release    # Release build (optimized for size)
cargo run --release      # Build and run
cargo test               # Run all tests
cargo test test_name     # Run specific test
cargo clippy --all-targets -- -D warnings   # Lint exactly as CI does (must be clean)
```

The crate is Windows-only. From WSL/Linux, add `--target x86_64-pc-windows-gnu`
to build, clippy and test; the test executables run through WSL interop.

### Benchmarking
`htop-win --benchmark=N [-d MS]` runs N refreshes and prints timing: collection,
draw (compose vs diff+output), rows repainted and bytes per frame, snapshot
pickup lag, and publish-to-frame latency (avg, max, p50, p99). Note the `=` in
`--benchmark=N`. For A/B comparisons between builds, add `-s PID`: with the
default CPU sort, tie order decides which processes are visible, so the two
builds would draw different content.

## Architecture

htop-win is a Windows htop clone with a custom TUI library. The codebase follows a clear separation:

### Core Modules

- **`main.rs`** - Entry point, terminal setup, event-driven main loop (see "Event Loop" below)
- **`app.rs`** - Application state (`App` struct), process sorting, tree building, dialogs (`DialogState` enum)
- **`data.rs`** - Background collector thread; publishes `SystemSnapshot`s through a capacity-one slot and takes back the process vec it replaced for reuse
- **`event_wait.rs`** - Waits on console input *or* a published snapshot (`WaitForMultipleObjects` on its own `CONIN$` handle plus an auto-reset event)
- **`input.rs`** - Keyboard and mouse event handling, dispatches to mode-specific handlers
- **`config.rs`** - Configuration struct (refresh rate, default settings)
- **`terminal.rs`** - Custom minimal TUI library replacing ratatui (~4000 lines incl. tests). Provides: Layout, Buffer, the retained `Compositor`, Terminal, Frame, and widgets (Block, Paragraph, Table, List, Scrollbar)
- **`numfmt.rs`** - Integer-math number formatting (byte-identical to `{:.1}`/`{:.0}`, without core::fmt's float machinery)
- **`json.rs`** - Minimal JSON parser for config files (replaces serde_json)

### System Module (`system/`)

- **`mod.rs`** - `SystemMetrics` struct aggregating all system data, refresh logic
- **`cpu.rs`** - CPU per-core usage via PDH performance counters (`\Processor Information(group,n)`), covering every processor group
- **`memory.rs`** - Memory/swap stats via Windows API, `format_bytes()` / `push_bytes()` helpers
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
- **`process_list.rs`** - Process table with sorting, selection, colors; painted row by row (no `Table` widget)
- **`text_pool.rs`** - Pooled strings for owned span text and vector recycling across lifetimes
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
Rendering is retained and row-granular:
- `Buffer` holds 32-byte cells (inline symbol, fg, bg, modifiers) plus per-row DIRTY/TOUCHED flags.
- The `Compositor` keeps its back buffer across frames. `Frame::paint_row(y, &RowSpec)` hashes exactly
  what paints the row (keyed SipHash). If the key matches the previous frame, the row is not touched at all.
  `Terminal::draw` then diffs only dirty rows against `front` (what the screen shows).
- `Layout.split()` handles `Constraint::Min` as flexible (expands to fill space)
- Widget styles: Apply background first, then render content to preserve span colors
- `Terminal` caches the console size. The main loop calls `refresh_size()` after each frame and on
  resize events, never before a draw: crossterm's `terminal::size()` costs ~80 µs per call on Windows.

### Base Layer vs Overlays (important when adding UI)
`ui::draw` calls `frame.begin(bg)`, paints the base layer (tab bar, header, process table, footer),
then `frame.finish_base()`. Everything drawn after that (dialogs, error banner) is an overlay.
- **Every base-layer write must go through `Frame::paint_row` / `Frame::paint_line`.** A direct
  buffer write or widget render in the base layer bypasses the row keys. It trips a debug assertion
  ("base-layer write outside Frame::paint_row"), and in release builds it forces a full repaint every frame.
- `RowSpec { x, width, base, segs }` must describe everything that paints the row, since it *is* the
  cache key. `RowSeg` takes a span slice, so callers can paint from a shared buffer (see
  `process_list::paint_table_row` and `header::paint_slots`).
- Overlays render normally with widgets. Rows they touch repaint on the next frame automatically.
- Keep frames allocation-free: owned span text comes from `ui::text_pool` (`pooled_fmt`, then
  `recycle_spans` after painting). Prefer fixed-size span arrays or reused buffers over a `Vec` per line.

### Event Loop (main.rs)
- Input first: drain crossterm with `event::poll(Duration::ZERO)`, then block on
  `EventWait::wait`. That wait returns on console input, on a published snapshot
  (`SnapshotReceiver::notify_on_publish`), or at the housekeeping timeout. Snapshot pickup is ~15-25 µs.
- The collector wakes early (`DataCollector::wake`) when the UI changes refresh rate, pause state or
  the metadata it needs.
- Timed waits that pace work use `event_wait::DeadlineWait` (a high-resolution waitable timer).
  Never use a `Condvar` or millisecond timeout for them: on Windows those round up to the
  ~15.6 ms timer tick, so every collection would start late.
- Startup: the collector's first collection (`SystemMetrics::refresh_initial`) skips the CPU counter
  sample. PDH setup costs ~300-400 ms and its first sample reads zero anyway. `prime_cpu()` runs
  right after the first snapshot is published, and the second sample follows 250 ms later, so real CPU%
  arrives ~0.8 s after launch. Keep slow one-time setup off the path to the first frame; `--benchmark` reports
  "First frame … after launch".
- Visible-row metadata: `enrich_viewport()` applies cached facts before the draw.
  `run_deferred_enrichment()` runs the Windows queries after the frame is out and requests a redraw.

### Display List (app.rs)
- `App::processes` is the collected list. The displayed rows are `DisplayRow`s that index into it
  (plus tree prefix/depth and the search flag). No per-snapshot clones.
- Invariant: `processes` is only replaced in `apply_snapshot`, which rebuilds the rows. It returns the
  replaced vec for the collector to reuse. A snapshot missing metadata the view needs is handed straight back.
- Read rows through `displayed(row)`, `displayed_len()`, `displayed_processes()` and `display_rows_from(start)`.
- Sorting: `sort_entry()` builds `(key, index)` pairs during the filter pass and `sort_order()` sorts them.
  Ties keep collector order. A new sortable column needs a key in `sort_key()`; the key-sort test covers every column.

### Render Verification
- `tests/retained_render.rs` is the differential gate. Retained frames must equal a from-scratch
  render cell for cell over a 2,000-step random walk, plus dirty-row budgets.
- Both sides of that gate use the same widgets and paint code, so it cannot catch a bug they share.
  Before rewriting a renderer, capture digests of full renders from the old code over a seeded random
  walk (in a scratch copy), then compare the new code against them.
- `tests/visual_test.rs` has reusable `App`/`ProcessInfo` fixtures that render with `ui::draw` into a `Buffer`.

## Dependencies

Minimal dependency set for small binary size:
- `crossterm` - Terminal events and manipulation
- `windows` - Direct Windows API bindings (no sysinfo wrapper)
- `bitflags` - Modifier flags for terminal styling
- `unicode-width` - Character width calculation
- `lexopt` - Lightweight argument parsing

## Releases & Auto-Update

### Creating a Release
1. Bump the version: `python3 bump-version.py patch` (updates `Cargo.toml` and `media/htop.rc`)
2. Commit and push changes; wait for CI (clippy with `-D warnings`, tests, release build) to pass
3. Create annotated tag; the tag message becomes the GitHub release notes:
   `git tag -a v0.0.X -F release-notes.md` (first line `htop-win v0.0.X`, then summary and sections)
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
