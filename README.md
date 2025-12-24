# htop-win

**A native Windows clone of [htop](https://htop.dev/) - the beloved interactive process viewer, rebuilt from the ground up in Rust.**

[![GitHub release](https://img.shields.io/github/v/release/faratech/htop-win?style=flat-square)](https://github.com/faratech/htop-win/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg?style=flat-square)](https://opensource.org/licenses/MIT)
![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11-0078D6?style=flat-square&logo=windows)
![Rust](https://img.shields.io/badge/rust-2024%20edition-orange?style=flat-square&logo=rust)
[![GitHub stars](https://img.shields.io/github/stars/faratech/htop-win?style=flat-square)](https://github.com/faratech/htop-win/stargazers)

> Inspired by the original [htop](https://github.com/htop-dev/htop) by Hisham Muhammad and contributors.

<p align="center">
  <img src="media/htop-cropped.png" alt="htop-win - Windows Process Viewer" width="100%">
</p>

<p align="center">
  <img src="images/demo_web.gif" alt="htop-win demo" width="100%">
</p>

---

## Why htop-win?

Windows Task Manager is fine, but power users deserve better. **htop-win** brings the beloved Unix htop experience to Windows with:

- **Blazing fast performance** - Direct Windows API calls, no wrappers
- **Tiny footprint** - ~500KB binary with minimal dependencies
- **Full htop compatibility** - Same keyboard shortcuts you already know
- **Windows-native features** - Efficiency Mode, CPU affinity, elevation detection
- **Automatic updates** - Background update checks with one-command upgrades

---

## Features

### Real-time System Monitoring
- Per-core CPU usage with color-coded gradient bars (green → yellow → red)
- Memory and swap usage visualization
- System uptime and task count display
- Configurable meter modes: Bar, Text, Graph, or Hidden

### Interactive Process Management
- **18 sortable columns**: PID, CPU%, MEM%, TIME+, Command, User, and more
- **Tree view**: Visualize parent-child process relationships with collapsible branches
- **Search & Filter**: Find processes instantly with live search
- **Process tagging**: Select multiple processes for batch operations
- **Kill processes**: Graceful termination or force kill
- **Priority control**: Change process priority class (Idle → Realtime)
- **CPU affinity**: Pin processes to specific CPU cores

### Windows-Specific Features
- **Efficiency Mode (EcoQoS)**: Reduce power consumption for background processes
- **Elevation detection**: See which processes run as Administrator
- **Architecture display**: Identify x86, x64, and ARM64 processes
- **Service account support**: View SYSTEM, LOCAL SERVICE, NETWORK SERVICE processes
- **Modified executable detection**: Highlight processes with changed/deleted binaries

### Full Mouse Support
- Click to select, double-click for details
- Right-click to tag processes
- Click column headers to sort
- Scroll wheel navigation
- Click meters to cycle display modes

### Customization
- **8 color themes**: Default, Monochrome, Light Terminal, Midnight, Nord, and more
- **Configurable columns**: Show/hide and reorder any column
- **Adjustable refresh rate**: 100ms to 5 seconds
- **Persistent settings**: Configuration saved to `%APPDATA%\htop-win\config`

---

## Installation

### Quick Install (Recommended)

Download the latest release for your architecture:

| Architecture | Download |
|--------------|----------|
| x64 (Intel/AMD) | [htop-win-amd64.exe](https://github.com/faratech/htop-win/releases/latest/download/htop-win-amd64.exe) |
| ARM64 | [htop-win-arm64.exe](https://github.com/faratech/htop-win/releases/latest/download/htop-win-arm64.exe) |

### Install to PATH

Run with administrator privileges to install globally:

```powershell
.\htop-win-amd64.exe --install
```

This installs to `%LOCALAPPDATA%\Microsoft\WindowsApps\htop.exe`, making `htop` available from any terminal.

### Update

Check for updates and install automatically:

```powershell
htop --update
```

Or force reinstall the current version:

```powershell
htop --update --force
```

### Build from Source

```powershell
# Clone the repository
git clone https://github.com/faratech/htop-win.git
cd htop-win

# Build optimized release (~500KB binary)
cargo build --release

# Run
.\target\release\htop-win.exe
```

**Requirements**: Rust 1.85+ (2024 edition), Windows 10/11

---

## Usage

```
htop-win [OPTIONS]

Options:
  -d, --delay <MS>          Refresh rate in milliseconds [default: 1500]
  -u, --user <USER>         Show only processes from this user
  -p, --pid <PID,...>       Show only these PIDs (comma-separated)
  -s, --sort <COLUMN>       Sort by: pid, cpu, mem, time, command, user
  -t, --tree                Start in tree view mode
  -F, --filter <STRING>     Initial filter string
  -H, --highlight <SECS>    Highlight new processes for N seconds
  -n, --iterations <N>      Exit after N updates (for scripting)
      --no-color            Monochrome mode
      --no-mouse            Disable mouse support
      --no-meters           Hide header meters
      --readonly            Disable kill/priority operations
      --install             Install to PATH (requires admin)
      --update              Check for and install updates
  -f, --force               Force install/update
  -h, --help                Show help
  -V, --version             Show version
```

### Examples

```powershell
# Monitor with 500ms refresh rate
htop -d 500

# Show only processes from current user
htop -u $env:USERNAME

# Start in tree view, sorted by memory
htop -t -s mem

# Filter to show only Chrome processes
htop -F chrome

# Monitor specific PIDs
htop -p 1234,5678,9012
```

---

## Keyboard Shortcuts

### Navigation

| Key | Action |
|-----|--------|
| `↑` / `k` | Move selection up |
| `↓` / `j` | Move selection down |
| `PgUp` / `PgDn` | Page up / down |
| `Home` / `g` | Jump to first process |
| `End` / `G` | Jump to last process |
| `Tab` | Cycle focus: Header → Process List → Footer |

### Main Controls (Function Keys)

| Key | Action |
|-----|--------|
| `F1` / `?` | Help screen |
| `F2` / `S` | Setup menu |
| `F3` / `/` | Search processes |
| `F4` / `\` | Filter processes |
| `F5` / `t` | Toggle tree view |
| `F6` / `<` `>` | Change sort column |
| `F7` / `]` | Decrease priority (nice+) |
| `F8` / `[` | Increase priority (nice-) |
| `F9` | Kill process |
| `F10` / `q` | Quit |

### Process Operations

| Key | Action |
|-----|--------|
| `Enter` | View process details |
| `Space` | Tag/untag process |
| `c` | Tag process and all children |
| `U` | Untag all processes |
| `Ctrl+T` | Tag all with same name |
| `Ctrl+A` | Toggle tag all visible |
| `e` | View environment variables |
| `w` | View full command line |
| `a` | Set CPU affinity |
| `F` | Toggle follow mode |

### Tree View

| Key | Action |
|-----|--------|
| `+` / `=` | Expand branch |
| `-` | Collapse branch |
| `*` | Toggle expand/collapse all |
| `Backspace` | Collapse to parent |

### Display

| Key | Action |
|-----|--------|
| `#` | Toggle header meters |
| `p` | Toggle program path display |
| `H` | Toggle user threads |
| `K` | Toggle kernel threads |
| `Z` | Pause/resume updates |
| `I` | Invert sort order |

### Quick Sort

| Key | Sort By |
|-----|---------|
| `P` | CPU% |
| `M` | Memory% |
| `T` | Time |
| `N` | PID |

---

## Configuration

htop-win saves settings to `%APPDATA%\htop-win\config\config.json`. Configure via the Setup menu (`F2`) or edit directly:

```json
{
  "refresh_rate_ms": 1500,
  "tree_view_default": false,
  "color_scheme": "Default",
  "show_kernel_threads": false,
  "show_program_path": false,
  "highlight_new_processes": true,
  "highlight_large_numbers": true,
  "cpu_meter_mode": "Bar",
  "memory_meter_mode": "Bar",
  "visible_columns": ["PID", "USER", "PRI", "CPU%", "MEM%", "TIME+", "Command"],
  "mouse_enabled": true,
  "confirm_kill": true
}
```

### Color Themes

Access via Setup (`F2`) → Color Scheme:

- **Default** - Classic htop colors
- **Monochrome** - No colors (accessibility)
- **Black on White** - Light theme
- **Light Terminal** - For light backgrounds
- **Midnight** - Dark blue theme
- **Blacknight** - Pure dark theme
- **Broken Gray** - Subtle grays
- **Nord** - Nord color palette

---

## Columns Reference

| Column | Description | Width |
|--------|-------------|-------|
| `PID` | Process ID | 7 |
| `PPID` | Parent Process ID | 7 |
| `USER` | Process owner | 10 |
| `PRI` | Priority (0-31) | 4 |
| `CLASS` | Priority class (Idle/Normal/High/Realtime) | 7 |
| `THR` | Thread count | 4 |
| `VIRT` | Virtual memory | 8 |
| `RES` | Resident (physical) memory | 8 |
| `SHR` | Shared memory | 8 |
| `S` | Status (R=Running, S=Sleeping) | 3 |
| `CPU%` | CPU usage percentage | 6 |
| `MEM%` | Memory usage percentage | 6 |
| `TIME+` | Cumulative CPU time | 10 |
| `START` | Process start time | 8 |
| `Command` | Command line | Flexible |
| `ELEV` | Elevated/Admin status | 4 |
| `ARCH` | Architecture (x86/x64/ARM64) | 5 |
| `ECO` | Efficiency Mode status | 4 |

---

## System Requirements

- **OS**: Windows 10 (1903+) or Windows 11
- **Architecture**: x64 (AMD64) or ARM64
- **Terminal**: Windows Terminal recommended (supports full color and Unicode)
- **Privileges**: Administrator recommended for full process visibility

---

## Dependencies

Minimal dependency set optimized for small binary size:

| Dependency | Purpose |
|------------|---------|
| [crossterm](https://github.com/crossterm-rs/crossterm) | Cross-platform terminal I/O |
| [windows-rs](https://github.com/microsoft/windows-rs) | Native Windows API bindings |
| [unicode-width](https://github.com/unicode-rs/unicode-width) | Unicode character width calculation |
| [bitflags](https://github.com/bitflags/bitflags) | Terminal modifier flags |
| [lexopt](https://github.com/blyber/lexopt) | Lightweight argument parsing |

**No heavy frameworks**: Custom terminal UI library (replaces ratatui), custom JSON parser (replaces serde).

---

## Performance

htop-win is designed for efficiency:

- **~500KB binary** - Minimal dependencies, LTO optimization
- **Efficiency Mode by default** - Runs with reduced CPU priority
- **Smart caching** - Minimizes Windows API calls
- **Diff-based rendering** - Only updates changed terminal cells
- **Direct API access** - No abstraction layers

Benchmark mode available: `htop --benchmark 100`

---

## Comparison with Alternatives

| Feature | htop-win | Task Manager | Process Explorer |
|---------|----------|--------------|------------------|
| Terminal-based | Yes | No | No |
| Keyboard-driven | Yes | Limited | Limited |
| Tree view | Yes | Yes | Yes |
| Process search | Yes | Yes | Yes |
| CPU affinity | Yes | Yes | Yes |
| Efficiency Mode | Yes | Yes | No |
| Custom columns | Yes | Limited | Yes |
| Color themes | 8 themes | No | No |
| Binary size | ~500KB | N/A | ~2MB |
| Auto-update | Yes | N/A | No |

---

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

---

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---

## Acknowledgments

- [htop](https://github.com/htop-dev/htop) - The original inspiration by Hisham Muhammad
- [windows-rs](https://github.com/microsoft/windows-rs) - Excellent Windows API bindings
- [crossterm](https://github.com/crossterm-rs/crossterm) - Cross-platform terminal library

---

<p align="center">
  <b>Star this repo if you find it useful!</b><br>
  <a href="https://github.com/faratech/htop-win">https://github.com/faratech/htop-win</a>
</p>
