mod app;
mod config;
mod input;
mod installer;
mod json;
mod system;
mod terminal;
mod ui;

use std::io;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use terminal::{CrosstermBackend, Terminal};

use app::App;
use config::Config;

/// Command-line arguments (parsed with lightweight lexopt)
#[derive(Debug, Default)]
struct Args {
    delay: Option<u64>,
    user: Option<String>,
    tree: bool,
    sort: Option<String>,
    no_mouse: bool,
    no_color: bool,
    pids: Option<Vec<u32>>,
    filter: Option<String>,
    max_iterations: Option<u64>,
    no_meters: bool,
    readonly: bool,
    highlight_changes: Option<u64>,
    help: bool,
    version: bool,
    benchmark: Option<u64>,
    inefficient: bool,
    install: bool,
}

/// Benchmark statistics for performance measurement
#[derive(Default)]
struct BenchmarkStats {
    refresh_times: Vec<Duration>,
    draw_times: Vec<Duration>,
    total_start: Option<Instant>,
    process_cpu_start: Duration,
}

fn parse_args() -> Result<Args, lexopt::Error> {
    use lexopt::prelude::*;

    let mut args = Args::default();
    let mut parser = lexopt::Parser::from_env();

    while let Some(arg) = parser.next()? {
        match arg {
            Short('d') | Long("delay") => {
                args.delay = Some(parser.value()?.parse()?);
            }
            Short('u') | Long("user") => {
                args.user = Some(parser.value()?.parse()?);
            }
            Short('t') | Long("tree") => {
                args.tree = true;
            }
            Short('s') | Long("sort") => {
                args.sort = Some(parser.value()?.parse()?);
            }
            Long("no-mouse") => {
                args.no_mouse = true;
            }
            Long("no-color") => {
                args.no_color = true;
            }
            Short('p') | Long("pid") => {
                let val: String = parser.value()?.parse()?;
                let pids: Vec<u32> = val
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                args.pids = Some(pids);
            }
            Short('F') | Long("filter") => {
                args.filter = Some(parser.value()?.parse()?);
            }
            Short('n') | Long("max-iterations") => {
                args.max_iterations = Some(parser.value()?.parse()?);
            }
            Long("no-meters") => {
                args.no_meters = true;
            }
            Long("readonly") => {
                args.readonly = true;
            }
            Short('H') | Long("highlight-changes") => {
                args.highlight_changes = Some(parser.value()?.parse()?);
            }
            Short('h') | Long("help") => {
                args.help = true;
            }
            Short('V') | Long("version") => {
                args.version = true;
            }
            Long("benchmark") => {
                args.benchmark = Some(parser.value().ok().and_then(|v| v.parse().ok()).unwrap_or(20));
            }
            Long("inefficient") => {
                args.inefficient = true;
            }
            Long("install") => {
                args.install = true;
            }
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(args)
}

fn print_help() {
    println!("htop-win {}", env!("CARGO_PKG_VERSION"));
    println!("Interactive process viewer for Windows\n");
    println!("USAGE: htop-win [OPTIONS]\n");
    println!("OPTIONS:");
    println!("  -d, --delay <MS>             Refresh rate in milliseconds (default: 1000)");
    println!("  -u, --user <USER>            Show only processes owned by USER");
    println!("  -t, --tree                   Start in tree view mode");
    println!("  -s, --sort <COLUMN>          Sort by: pid, cpu, mem, time, command, user");
    println!("      --no-mouse               Disable mouse support");
    println!("      --no-color               Use monochrome mode");
    println!("  -p, --pid <PID,...>          Show only specific PIDs (comma-separated)");
    println!("  -F, --filter <FILTER>        Initial filter string");
    println!("  -n, --max-iterations <N>     Exit after N updates");
    println!("      --no-meters              Hide header meters");
    println!("      --benchmark [N]          Run N iterations (default 20) and print timing stats");
    println!("      --readonly               Disable kill/priority operations");
    println!("      --inefficient            Disable Efficiency Mode (run at normal priority)");
    println!("  -H, --highlight-changes <S>  Highlight process changes (seconds)");
    println!("      --install                Install to PATH (requires admin, will prompt UAC)");
    println!("  -h, --help                   Print help");
    println!("  -V, --version                Print version");
}

/// Get current process CPU time (user + kernel) on Windows
#[cfg(windows)]
fn get_process_cpu_time() -> Duration {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    unsafe {
        let handle = GetCurrentProcess();
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();

        if GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user).is_ok() {
            let kernel_100ns = ((kernel.dwHighDateTime as u64) << 32) | kernel.dwLowDateTime as u64;
            let user_100ns = ((user.dwHighDateTime as u64) << 32) | user.dwLowDateTime as u64;
            let total_100ns = kernel_100ns + user_100ns;
            Duration::from_nanos(total_100ns * 100)
        } else {
            Duration::ZERO
        }
    }
}

#[cfg(not(windows))]
fn get_process_cpu_time() -> Duration {
    Duration::ZERO
}

/// Enable Windows Efficiency Mode (EcoQoS) for the current process
/// This reduces CPU usage by lowering priority and enabling power throttling
#[cfg(windows)]
fn enable_efficiency_mode() {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, SetPriorityClass, SetProcessInformation,
        ProcessPowerThrottling, IDLE_PRIORITY_CLASS,
        PROCESS_POWER_THROTTLING_STATE, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    };

    unsafe {
        let handle = GetCurrentProcess();

        // Set to idle priority class (lowest scheduling priority)
        let _ = SetPriorityClass(handle, IDLE_PRIORITY_CLASS);

        // Enable EcoQoS power throttling
        let mut throttle_state = PROCESS_POWER_THROTTLING_STATE {
            Version: 1, // PROCESS_POWER_THROTTLING_CURRENT_VERSION
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
                | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
            StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
                | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        };

        let _ = SetProcessInformation(
            handle,
            ProcessPowerThrottling,
            &mut throttle_state as *mut _ as *mut _,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
    }
}

#[cfg(not(windows))]
fn enable_efficiency_mode() {
    // No-op on non-Windows platforms
}

impl BenchmarkStats {
    fn new() -> Self {
        Self {
            refresh_times: Vec::new(),
            draw_times: Vec::new(),
            total_start: Some(Instant::now()),
            process_cpu_start: get_process_cpu_time(),
        }
    }

    fn record_refresh(&mut self, duration: Duration) {
        self.refresh_times.push(duration);
    }

    fn record_draw(&mut self, duration: Duration) {
        self.draw_times.push(duration);
    }

    fn print_report(&self, process_count: usize) {
        let total_elapsed = self.total_start.map(|s| s.elapsed()).unwrap_or_default();
        let process_cpu_end = get_process_cpu_time();
        let process_cpu_used = process_cpu_end.saturating_sub(self.process_cpu_start);

        // Calculate CPU percentage (CPU time / wall time * 100)
        let cpu_percent = if total_elapsed.as_nanos() > 0 {
            (process_cpu_used.as_nanos() as f64 / total_elapsed.as_nanos() as f64) * 100.0
        } else {
            0.0
        };

        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!("║                    BENCHMARK RESULTS                         ║");
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║ Iterations: {:>6}    Processes: {:>6}                       ║",
                 self.refresh_times.len(), process_count);
        println!("╠══════════════════════════════════════════════════════════════╣");

        // Refresh stats
        if !self.refresh_times.is_empty() {
            let avg = self.refresh_times.iter().sum::<Duration>() / self.refresh_times.len() as u32;
            let min = self.refresh_times.iter().min().copied().unwrap_or_default();
            let max = self.refresh_times.iter().max().copied().unwrap_or_default();
            let total: Duration = self.refresh_times.iter().sum();
            println!("║ REFRESH (system data collection)                             ║");
            println!("║   Total: {:>10.2?}  Avg: {:>10.2?}                       ║", total, avg);
            println!("║   Min:   {:>10.2?}  Max: {:>10.2?}                       ║", min, max);
        }

        // Draw stats
        if !self.draw_times.is_empty() {
            let avg = self.draw_times.iter().sum::<Duration>() / self.draw_times.len() as u32;
            let min = self.draw_times.iter().min().copied().unwrap_or_default();
            let max = self.draw_times.iter().max().copied().unwrap_or_default();
            let total: Duration = self.draw_times.iter().sum();
            println!("╠══════════════════════════════════════════════════════════════╣");
            println!("║ DRAW (UI rendering)                                          ║");
            println!("║   Total: {:>10.2?}  Avg: {:>10.2?}                       ║", total, avg);
            println!("║   Min:   {:>10.2?}  Max: {:>10.2?}                       ║", min, max);
        }

        // Overall stats
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║ OVERALL                                                      ║");
        println!("║   Wall time:    {:>10.2?}                                  ║", total_elapsed);
        println!("║   CPU time:     {:>10.2?}                                  ║", process_cpu_used);
        println!("║   CPU usage:    {:>10.1}%                                  ║", cpu_percent);
        println!("╚══════════════════════════════════════════════════════════════╝");
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = match parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    if args.help {
        print_help();
        return Ok(());
    }

    if args.version {
        println!("htop-win {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if args.install {
        if let Err(e) = installer::install_to_path() {
            eprintln!("Installation failed: {}", e);
            std::process::exit(1);
        }
        return Ok(());
    }

    // Enable Efficiency Mode by default (reduces CPU usage via EcoQoS)
    if !args.inefficient {
        enable_efficiency_mode();
    }

    // Enable SeDebugPrivilege to access service account info (NETWORK SERVICE, LOCAL SERVICE)
    // Only succeeds when running as Administrator
    system::enable_debug_privilege();

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    if args.no_mouse {
        execute!(stdout, EnterAlternateScreen)?;
    } else {
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Drain any pending input events to prevent stray keypresses on startup
    while event::poll(Duration::from_millis(10))? {
        let _ = event::read();
    }

    // Load configuration from file (or use defaults)
    let mut config = Config::load();

    // Apply command-line overrides
    if let Some(delay) = args.delay {
        config.refresh_rate_ms = delay;
    }
    if args.tree {
        config.tree_view_default = true;
    }
    if args.no_color {
        config.color_scheme = ui::colors::ColorScheme::Monochrome;
    }
    if args.readonly {
        config.readonly = true;
    }
    if let Some(delay) = args.highlight_changes {
        config.highlight_new_processes = true;
        config.highlight_duration_ms = delay * 1000;
    }

    let mut app = App::new(config.clone());

    // Apply user filter from CLI
    if let Some(ref user) = args.user {
        app.user_filter = Some(user.clone());
    }

    // Apply sort column from CLI
    if let Some(ref sort) = args.sort {
        app.sort_column = match sort.to_lowercase().as_str() {
            "pid" => app::SortColumn::Pid,
            "cpu" | "cpu%" => app::SortColumn::Cpu,
            "mem" | "mem%" | "memory" => app::SortColumn::Mem,
            "time" => app::SortColumn::Time,
            "command" | "cmd" => app::SortColumn::Command,
            "user" => app::SortColumn::User,
            "ppid" => app::SortColumn::PPid,
            "threads" | "thr" => app::SortColumn::Threads,
            _ => app::SortColumn::Cpu,
        };
    }

    // Apply filter from CLI
    if let Some(ref filter) = args.filter {
        app.filter_string = filter.clone();
        app.filter_string_lower = filter.to_lowercase();
    }

    // Apply PID filter from CLI (convert Vec to HashSet for O(1) lookup)
    if let Some(ref pids) = args.pids {
        app.pid_filter = Some(pids.iter().copied().collect());
    }

    // Apply max iterations from CLI
    if let Some(n) = args.max_iterations {
        app.max_iterations = Some(n);
    }

    // Apply no-meters from CLI
    if args.no_meters {
        app.show_header = false;
    }

    // Setup benchmark mode if requested
    let benchmark_mode = args.benchmark;
    if let Some(n) = benchmark_mode {
        app.max_iterations = Some(n);
        // Use minimal delay in benchmark mode for faster iteration
        app.config.refresh_rate_ms = 10;
    }

    // Initial system refresh
    app.refresh_system();
    let process_count = app.processes.len();

    // Create benchmark stats if in benchmark mode
    let mut bench_stats = benchmark_mode.map(|_| BenchmarkStats::new());

    // Run the main loop
    let result = run_app(&mut terminal, &mut app, &config, bench_stats.as_mut());

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = &result {
        eprintln!("Error: {err:?}");
    }

    // Print benchmark report if in benchmark mode
    if let Some(stats) = bench_stats {
        stats.print_report(process_count);
    }

    Ok(())
}

fn run_app(
    terminal: &mut Terminal,
    app: &mut App,
    _config: &Config,
    mut bench_stats: Option<&mut BenchmarkStats>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_tick = Instant::now();
    let mut needs_redraw = true;

    loop {
        // Read tick rate from app.config so it updates dynamically
        let tick_rate = Duration::from_millis(app.config.refresh_rate_ms);

        // Draw UI only when needed (state changed)
        if needs_redraw {
            let draw_start = Instant::now();
            terminal.draw(|f| ui::draw(f, app))?;
            if let Some(stats) = bench_stats.as_mut() {
                stats.record_draw(draw_start.elapsed());
            }
            needs_redraw = false;
        }

        // Handle events with timeout
        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    if input::handle_key_event(app, key) {
                        return Ok(());
                    }
                    needs_redraw = true;
                }
                Event::Mouse(mouse) => {
                    input::handle_mouse_event(app, mouse);
                    needs_redraw = true;
                }
                Event::Resize(_, _) => {
                    // Terminal will handle resize automatically
                    needs_redraw = true;
                }
                _ => {}
            }
        }

        // Refresh system data at tick rate (unless paused)
        if last_tick.elapsed() >= tick_rate {
            if !app.paused {
                let refresh_start = Instant::now();
                app.refresh_system();
                if let Some(stats) = bench_stats.as_mut() {
                    stats.record_refresh(refresh_start.elapsed());
                }
                app.iteration_count += 1;
                needs_redraw = true;

                // Check if we've reached max iterations
                if let Some(max) = app.max_iterations
                    && app.iteration_count >= max {
                        return Ok(());
                    }
            }

            // Refresh I/O counters when process info dialog is open (even when paused)
            if app.view_mode == app::ViewMode::ProcessInfo {
                app.refresh_process_info_io();
                needs_redraw = true;
            }

            // Advance the tick even while paused to avoid busy-looping with a
            // zero-duration poll timeout (which drives CPU usage up).
            last_tick = Instant::now();
        }
    }
}
