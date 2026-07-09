// The binary is a thin wrapper over the htop_win library crate — modules are
// declared once in lib.rs so each source file compiles a single time.
use htop_win::{app, config, data, input, installer, system, terminal, ui};

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossterm::{
    cursor,
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
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
    update: bool,
    force: bool,
    gpu_debug: bool,
    cpu_debug: bool,
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
                let mut pids = Vec::new();
                for part in val.split(',') {
                    let pid = part
                        .trim()
                        .parse()
                        .map_err(|_| lexopt::Error::from(format!("Invalid PID: {part}")))?;
                    pids.push(pid);
                }
                args.pids = Some(pids);
            }
            Short('F') | Long("filter") => {
                args.filter = Some(parser.value()?.parse()?);
            }
            Short('n') | Long("max-iterations") | Long("iterations") => {
                args.max_iterations = Some(parser.value()?.parse()?);
            }
            Long("no-meters") => {
                args.no_meters = true;
            }
            Long("readonly") => {
                args.readonly = true;
            }
            Short('H') | Long("highlight-changes") | Long("highlight") => {
                args.highlight_changes = Some(parser.value()?.parse()?);
            }
            Short('h') | Long("help") => {
                args.help = true;
            }
            Short('V') | Long("version") => {
                args.version = true;
            }
            Long("benchmark") => {
                args.benchmark = Some(match parser.optional_value() {
                    Some(value) => value.parse()?,
                    None => 20,
                });
            }
            Long("benchmark-iterations") => {
                args.benchmark = Some(parser.value()?.parse()?);
            }
            Long("inefficient") => {
                args.inefficient = true;
            }
            Long("install") => {
                args.install = true;
            }
            Long("update") => {
                args.update = true;
            }
            Long("force") | Short('f') => {
                args.force = true;
            }
            Long("gpu-debug") => {
                args.gpu_debug = true;
            }
            Long("cpu-debug") => {
                args.cpu_debug = true;
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
    println!("  -d, --delay <MS>             Refresh rate in milliseconds (default: 1500)");
    println!("  -u, --user <USER>            Show only processes owned by USER");
    println!("  -t, --tree                   Start in tree view mode");
    println!("  -s, --sort <COLUMN>          Sort by: pid, ppid, cpu/cpu%, mem/mem%/memory,");
    println!("                               time, command/cmd, user, threads/thr");
    println!("      --no-mouse               Disable mouse support");
    println!("      --no-color               Use monochrome mode");
    println!("  -p, --pid <PID,...>          Show only specific PIDs (comma-separated)");
    println!("  -F, --filter <FILTER>        Initial filter string");
    println!("  -n, --max-iterations <N>     Exit after N updates (alias: --iterations)");
    println!("      --no-meters              Hide header meters");
    println!("      --benchmark[=<N>]        Run N iterations (default 20) and print timing stats");
    println!("      --benchmark-iterations <N>  Alias with a separate iteration value");
    println!("      --readonly               Disable process mutation operations");
    println!("      --inefficient            Disable Efficiency Mode (run at normal priority)");
    println!(
        "  -H, --highlight-changes <S>  Highlight process changes (seconds; alias: --highlight)"
    );
    println!("      --install                Install for the current user and add it to PATH");
    println!("      --update                 Check for updates and install if available");
    println!("  -f, --force                  Force install/update even if same version");
    println!("      --gpu-debug              Print GPU/NPU adapter diagnostics and exit");
    println!("      --cpu-debug              Print CPU / processor-group diagnostics and exit");
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
        GetCurrentProcess, IDLE_PRIORITY_CLASS, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
        ProcessPowerThrottling, SetPriorityClass, SetProcessInformation,
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
        println!(
            "║ Iterations: {:>6}    Processes: {:>6}                       ║",
            self.refresh_times.len(),
            process_count
        );
        println!("╠══════════════════════════════════════════════════════════════╣");

        // Refresh stats
        if !self.refresh_times.is_empty() {
            let avg = self.refresh_times.iter().sum::<Duration>() / self.refresh_times.len() as u32;
            let min = self.refresh_times.iter().min().copied().unwrap_or_default();
            let max = self.refresh_times.iter().max().copied().unwrap_or_default();
            let total: Duration = self.refresh_times.iter().sum();
            println!("║ REFRESH (system data collection)                             ║");
            println!(
                "║   Total: {:>10.2?}  Avg: {:>10.2?}                       ║",
                total, avg
            );
            println!(
                "║   Min:   {:>10.2?}  Max: {:>10.2?}                       ║",
                min, max
            );
        }

        // Draw stats
        if !self.draw_times.is_empty() {
            let avg = self.draw_times.iter().sum::<Duration>() / self.draw_times.len() as u32;
            let min = self.draw_times.iter().min().copied().unwrap_or_default();
            let max = self.draw_times.iter().max().copied().unwrap_or_default();
            let total: Duration = self.draw_times.iter().sum();
            println!("╠══════════════════════════════════════════════════════════════╣");
            println!("║ DRAW (UI rendering)                                          ║");
            println!(
                "║   Total: {:>10.2?}  Avg: {:>10.2?}                       ║",
                total, avg
            );
            println!(
                "║   Min:   {:>10.2?}  Max: {:>10.2?}                       ║",
                min, max
            );
        }

        // Overall stats
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║ OVERALL                                                      ║");
        println!(
            "║   Wall time:    {:>10.2?}                                  ║",
            total_elapsed
        );
        println!(
            "║   CPU time:     {:>10.2?}                                  ║",
            process_cpu_used
        );
        println!(
            "║   CPU usage:    {:>10.1}%                                  ║",
            cpu_percent
        );
        println!("╚══════════════════════════════════════════════════════════════╝");
    }
}

/// True once mouse capture has been enabled, so restore only disables what was set.
static MOUSE_CAPTURE_ENABLED: AtomicBool = AtomicBool::new(false);

/// Restore the terminal to its normal state. Idempotent and infallible so it is
/// safe to call from the panic hook, the error path, and the normal exit path
/// alike (errors are ignored — there is nothing useful to do with them).
fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut stdout = io::stdout();
    let _ = execute!(stdout, LeaveAlternateScreen);
    if MOUSE_CAPTURE_ENABLED.load(Ordering::Relaxed) {
        let _ = execute!(stdout, DisableMouseCapture);
    }
    let _ = execute!(stdout, cursor::Show);
}

fn load_session_config(args: &Args) -> (Config, bool) {
    // Load this before terminal setup because mouse capture is itself a
    // persisted setting, not merely an in-app rendering preference.
    let first_run = !Config::config_path().is_some_and(|path| path.exists());
    let mut config = Config::load();
    apply_config_overrides(&mut config, args);

    (config, first_run)
}

fn apply_config_overrides(config: &mut Config, args: &Args) {
    if let Some(delay) = args.delay {
        config.refresh_rate_ms = delay.max(100);
    }
    if args.tree {
        config.tree_view_default = true;
    }
    if args.no_color {
        config.color_scheme = ui::colors::ColorScheme::Monochrome;
    }
    if let Some(delay) = args.highlight_changes {
        config.highlight_new_processes = true;
        config.highlight_duration_ms = delay.saturating_mul(1000);
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

    if args.gpu_debug {
        print!("{}", system::gpu_debug_dump());
        return Ok(());
    }

    if args.cpu_debug {
        print!("{}", system::cpu_debug_dump());
        return Ok(());
    }

    if args.install {
        if let Err(e) = installer::install_to_path(args.force) {
            eprintln!("Installation failed: {}", e);
            std::process::exit(1);
        }
        return Ok(());
    }

    if args.update {
        if let Err(e) = installer::update_from_github(args.force) {
            eprintln!("Update failed: {}", e);
            std::process::exit(1);
        }
        return Ok(());
    }

    // Apply any pending update before starting (downloaded in previous session)
    let update_just_applied = installer::apply_pending_update();

    // Enable Efficiency Mode by default (reduces CPU usage via EcoQoS)
    if !args.inefficient {
        enable_efficiency_mode();
    }

    // Enable SeDebugPrivilege to access service account info (NETWORK SERVICE, LOCAL SERVICE)
    // Only succeeds when running as Administrator
    system::enable_debug_privilege();

    let (config, first_run) = load_session_config(&args);
    let mouse_enabled = config.mouse_enabled && !args.no_mouse;

    // Restore the terminal before the default panic handler prints, so the
    // message lands on the normal screen instead of the soon-to-vanish
    // alternate one. Panic hooks still run before abort() under the release
    // profile's panic = "abort". (In debug builds a background-thread panic
    // restores while the UI thread keeps drawing; in release the process
    // aborts immediately, so the hook's view is exact. In debug builds, make a
    // background panic process-fatal too; continuing the UI after its collector
    // or updater died would leave a deceptively frozen application.)
    let main_thread = std::thread::current().id();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let background_panic = std::thread::current().id() != main_thread;
        restore_terminal();
        default_hook(info);
        if background_panic {
            std::process::abort();
        }
    }));

    // Setup terminal. If this very first step fails there is nothing to
    // restore; every failure after it returns through run_tui to the single
    // restore_terminal() call below.
    enable_raw_mode()?;

    let (result, bench_stats, process_count) =
        run_tui(&args, update_just_applied, config, first_run, mouse_enabled);

    // Restore terminal — the one restore point for both Ok and error returns
    restore_terminal();

    if let Err(err) = &result {
        eprintln!("Error: {err:?}");
    }

    // Print benchmark report if in benchmark mode
    if let Some(stats) = bench_stats {
        stats.print_report(process_count);
    }

    if result.is_err() {
        std::process::exit(1);
    }

    Ok(())
}

/// Run the TUI session, returning the run result together with the benchmark
/// state and process count needed for the post-restore report (which must be
/// delivered even when the session ends in an error).
fn run_tui(
    args: &Args,
    update_just_applied: bool,
    config: Config,
    first_run: bool,
    mouse_enabled: bool,
) -> (
    Result<(), Box<dyn std::error::Error>>,
    Option<BenchmarkStats>,
    usize,
) {
    let mut bench_stats = None;
    let mut process_count = 0;
    let result = run_tui_inner(
        args,
        update_just_applied,
        config,
        first_run,
        mouse_enabled,
        &mut bench_stats,
        &mut process_count,
    );
    (result, bench_stats, process_count)
}

/// Everything between raw-mode setup and terminal restore. Any `?` failure in
/// here propagates back to main(), which restores the terminal exactly once.
fn run_tui_inner(
    args: &Args,
    update_just_applied: bool,
    config: Config,
    first_run: bool,
    mouse_enabled: bool,
    bench_stats: &mut Option<BenchmarkStats>,
    process_count: &mut usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut stdout = io::stdout();
    if mouse_enabled {
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        MOUSE_CAPTURE_ENABLED.store(true, Ordering::Relaxed);
    } else {
        execute!(stdout, EnterAlternateScreen)?;
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Drain any pending input events to prevent stray keypresses on startup
    while event::poll(Duration::from_millis(10))? {
        let _ = event::read();
    }

    let mut app = App::new(config);
    app.set_runtime_readonly(args.readonly);
    app.update_checked = update_just_applied;

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
            _ => return Err(format!("Unknown sort column: {sort}").into()),
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

    // Spawn background data collector and wait for initial snapshot
    let (collector, data_rx) = data::DataCollector::spawn_with_enrichment(
        app.config.refresh_rate_ms,
        app.canonical_enrichment_requirements(),
    );
    if let Ok(snapshot) = data_rx.recv() {
        app.apply_snapshot(snapshot);
    }
    if first_run {
        app.apply_hardware_default_columns();
    }
    *process_count = app.processes.len();

    // Create benchmark stats if in benchmark mode
    *bench_stats = benchmark_mode.map(|_| BenchmarkStats::new());

    // Spawn background update check (skip if we just applied an update, since
    // the running binary is still the old version and would re-download)
    let update_rx = if update_just_applied {
        // Create a dummy channel that never sends anything
        let (_, rx) = std::sync::mpsc::channel();
        rx
    } else {
        installer::spawn_update_check()
    };

    // Run the main loop
    let result = run_app(
        &mut terminal,
        &mut app,
        bench_stats.as_mut(),
        update_rx,
        data_rx,
        &collector,
    );

    // Persist any config change still pending from the debounced hot paths
    // (meter clicks / arrow-key meter cycling).
    if !app.retry_config_save() && result.is_ok() {
        return Err("Failed to save configuration".into());
    }

    result
}

fn run_app(
    terminal: &mut Terminal,
    app: &mut App,
    mut bench_stats: Option<&mut BenchmarkStats>,
    update_rx: std::sync::mpsc::Receiver<installer::UpdateStatus>,
    data_rx: data::SnapshotReceiver,
    collector: &data::DataCollector,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_tick = Instant::now();
    let mut needs_redraw = true;

    loop {
        collector.set_enrichment_requirements(app.canonical_enrichment_requirements());

        // Read tick rate from app.config so it updates dynamically
        let tick_rate = Duration::from_millis(app.config.refresh_rate_ms);

        // Flush deferred process list update before rendering
        if app.needs_process_update {
            app.update_displayed_processes();
            app.needs_process_update = false;
            needs_redraw = true;
        }

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
                    // Sync shared state with background collector
                    collector.paused.store(app.paused, Ordering::Relaxed);
                    collector
                        .tick_rate_ms
                        .store(app.config.refresh_rate_ms, Ordering::Relaxed);
                    needs_redraw = true;
                }
                Event::Mouse(mouse) => {
                    if input::handle_mouse_event(app, mouse) {
                        return Ok(());
                    }
                    // Sync shared state with background collector
                    collector.paused.store(app.paused, Ordering::Relaxed);
                    collector
                        .tick_rate_ms
                        .store(app.config.refresh_rate_ms, Ordering::Relaxed);
                    needs_redraw = true;
                }
                Event::Resize(_, _) => {
                    // Terminal will handle resize automatically
                    needs_redraw = true;
                }
                _ => {}
            }
        }

        // Check for update result from background thread
        if !app.update_checked
            && let Ok(status) = update_rx.try_recv()
        {
            app.update_checked = true;
            match status {
                installer::UpdateStatus::Downloaded { version, path } => {
                    app.update_available = Some((version.clone(), path));
                    app.status_message = Some((
                        format!("Update v{} downloaded. Restart to apply.", version),
                        Instant::now(),
                    ));
                }
                installer::UpdateStatus::UpToDate => {
                    app.status_message = Some((
                        format!("v{} Up-to-date!", env!("CARGO_PKG_VERSION")),
                        Instant::now(),
                    ));
                }
                installer::UpdateStatus::Failed(error) => {
                    app.status_message =
                        Some((format!("Update check failed: {error}"), Instant::now()));
                }
            }
            needs_redraw = true;
        }

        // The collector's capacity-one slot has already discarded superseded
        // snapshots, so one non-blocking receive is always the newest state.
        {
            if let Ok(snapshot) = data_rx.try_recv() {
                if let Some(stats) = bench_stats.as_mut() {
                    stats.record_refresh(snapshot.refresh_duration);
                }
                // Recycle old vec before replacing
                let old = std::mem::take(&mut app.processes);
                let _ = collector.recycle_tx.send(old);
                app.apply_snapshot(snapshot);
                app.iteration_count += 1;
                needs_redraw = true;

                // Check if we've reached max iterations
                if let Some(max) = app.max_iterations
                    && app.iteration_count >= max
                {
                    return Ok(());
                }
            }
        }

        // Refresh I/O counters when process info dialog is open (at tick rate, even when paused)
        if last_tick.elapsed() >= tick_rate {
            if matches!(app.dialog, app::DialogState::ProcessInfo { .. }) {
                app.refresh_process_info_io();
                needs_redraw = true;
            }

            // Flush debounced config changes at most once per tick.
            app.flush_config();

            // Advance the tick even while paused to avoid busy-looping with a
            // zero-duration poll timeout (which drives CPU usage up).
            last_tick = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_readonly_is_not_written_into_persisted_config() {
        let args = Args {
            readonly: true,
            ..Args::default()
        };
        let mut config = Config::default();

        apply_config_overrides(&mut config, &args);

        assert!(!config.readonly);
    }
}
