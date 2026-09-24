// The binary is a thin wrapper over the htop_win library crate — modules are
// declared once in lib.rs so each source file compiles a single time.
use htop_win::{app, config, data, event_wait, input, installer, system, terminal, ui};

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
    /// Per-frame split of each draw (compose vs. diff + console output) and
    /// how many rows the retained frame actually repainted.
    frames: Vec<terminal::FrameStats>,
    /// Publish-to-apply delay of each snapshot (UI pickup latency).
    snapshot_lags: Vec<Duration>,
    /// Publish-to-drawn delay of each snapshot (pickup + apply + draw).
    frame_latencies: Vec<Duration>,
    /// Snapshots the collector replaced before the UI took them.
    superseded_snapshots: u64,
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
    println!("                               (refresh 10 ms unless -d is given)");
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
            frames: Vec::new(),
            snapshot_lags: Vec::new(),
            frame_latencies: Vec::new(),
            superseded_snapshots: 0,
            total_start: Some(Instant::now()),
            process_cpu_start: get_process_cpu_time(),
        }
    }

    fn record_refresh(&mut self, duration: Duration) {
        self.refresh_times.push(duration);
    }

    fn record_draw(&mut self, duration: Duration, frame: terminal::FrameStats) {
        self.draw_times.push(duration);
        self.frames.push(frame);
    }

    fn record_snapshot_lag(&mut self, lag: Duration) {
        self.snapshot_lags.push(lag);
    }

    fn record_frame_latency(&mut self, latency: Duration) {
        self.frame_latencies.push(latency);
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
                "║   Total: {:>10}  Avg: {:>10}                       ║",
                bench_ms(total),
                bench_ms(avg)
            );
            println!(
                "║   Min:   {:>10}  Max: {:>10}                       ║",
                bench_ms(min),
                bench_ms(max)
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
                "║   Total: {:>10}  Avg: {:>10}                       ║",
                bench_ms(total),
                bench_ms(avg)
            );
            println!(
                "║   Min:   {:>10}  Max: {:>10}                       ║",
                bench_ms(min),
                bench_ms(max)
            );
            let frames = self.frames.len().max(1);
            let compose: Vec<Duration> = self.frames.iter().map(|f| f.compose).collect();
            let output: Vec<Duration> = self.frames.iter().map(|f| f.output).collect();
            let rows: usize = self.frames.iter().map(|f| usize::from(f.dirty_rows)).sum();
            let bytes: usize = self.frames.iter().map(|f| f.bytes).sum();
            println!(
                "║   Compose avg: {:>10}  Diff+output avg: {:>10}      ║",
                bench_ms(avg_max(&compose).0),
                bench_ms(avg_max(&output).0)
            );
            println!(
                "║   Rows repainted/frame: {:>5}  Bytes/frame: {:>7}          ║",
                format!("{}.{}", rows / frames, rows * 10 / frames % 10),
                bytes / frames
            );
        }

        // Snapshot latency: publish → UI pickup, and publish → frame drawn
        if !self.snapshot_lags.is_empty() {
            let (pickup_avg, pickup_max) = avg_max(&self.snapshot_lags);
            let (frame_avg, frame_max) = avg_max(&self.frame_latencies);
            println!("╠══════════════════════════════════════════════════════════════╣");
            println!("║ SNAPSHOT LATENCY (from collector publish)                    ║");
            println!(
                "║   Pickup Avg: {:>10}  Max: {:>10}                  ║",
                bench_ms(pickup_avg),
                bench_ms(pickup_max)
            );
            println!(
                "║   Frame  Avg: {:>10}  Max: {:>10}                  ║",
                bench_ms(frame_avg),
                bench_ms(frame_max)
            );
            println!(
                "║   Frame  p50: {:>10}  p99: {:>10}                  ║",
                bench_ms(percentile(&self.frame_latencies, 50)),
                bench_ms(percentile(&self.frame_latencies, 99))
            );
            println!(
                "║   Dropped (superseded) snapshots: {:>6}                     ║",
                self.superseded_snapshots
            );
        }

        // Overall stats
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║ OVERALL                                                      ║");
        println!(
            "║   Wall time:    {:>10}                                  ║",
            bench_ms(total_elapsed)
        );
        println!(
            "║   CPU time:     {:>10}                                  ║",
            bench_ms(process_cpu_used)
        );
        // Integer tenths keep core::fmt's float machinery out of the binary.
        let cpu_tenths = (cpu_percent * 10.0).round_ties_even() as i64;
        println!(
            "║   CPU usage:    {:>10}%                                  ║",
            format!("{}{}{}", cpu_tenths / 10, ".", cpu_tenths % 10)
        );
        println!("╚══════════════════════════════════════════════════════════════╝");
    }
}

/// Format a Duration as milliseconds with three decimals ("7.081ms"),
/// computed with integer math so the benchmark report does not link
/// Duration's precision-aware Debug formatting.
fn bench_ms(duration: Duration) -> String {
    let micros = duration.as_micros();
    format!("{}.{:03}ms", micros / 1000, micros % 1000)
}

/// Average and maximum of a sample set (zero for an empty set).
/// Nearest-rank percentile (`pct` in 0..=100) of the samples.
fn percentile(samples: &[Duration], pct: usize) -> Duration {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = (sorted.len() * pct).div_ceil(100).max(1);
    sorted.get(rank - 1).copied().unwrap_or_default()
}

fn avg_max(samples: &[Duration]) -> (Duration, Duration) {
    let max = samples.iter().max().copied().unwrap_or_default();
    let avg = samples
        .iter()
        .sum::<Duration>()
        .checked_div(samples.len() as u32)
        .unwrap_or_default();
    (avg, max)
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

/// Extract the panic message without std's default-hook machinery (release
/// hook only). Non-string payloads fall back to a fixed placeholder.
#[cfg(not(debug_assertions))]
fn panic_message<'a>(info: &'a std::panic::PanicHookInfo<'_>) -> &'a str {
    if let Some(message) = info.payload().downcast_ref::<&str>() {
        message
    } else if let Some(message) = info.payload().downcast_ref::<String>() {
        message.as_str()
    } else {
        "Box<dyn Any>"
    }
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

    // Restore the terminal before the panic message prints, so it lands on
    // the normal screen instead of the soon-to-vanish alternate one. Panic
    // hooks still run before abort() under the release profile's panic =
    // "abort". (In debug builds a background-thread panic restores while the
    // UI thread keeps drawing; in release the process aborts immediately, so
    // the hook's view is exact. In debug builds, make a background panic
    // process-fatal too; continuing the UI after its collector or updater
    // died would leave a deceptively frozen application.)
    //
    // Release uses a self-contained hook: chaining to the default hook would
    // link std's panic formatter (thread names, backtrace note) into the
    // binary for a message this small.
    let main_thread = std::thread::current().id();
    #[cfg(not(debug_assertions))]
    {
        use std::io::Write as _;
        std::panic::set_hook(Box::new(move |info| {
            let background_panic = std::thread::current().id() != main_thread;
            restore_terminal();
            let mut err = io::stderr().lock();
            let _ = writeln!(err, "htop-win panicked: {}", panic_message(info));
            if let Some(location) = info.location() {
                let _ = writeln!(err, "at {location}");
            }
            if background_panic {
                std::process::abort();
            }
        }));
    }
    #[cfg(debug_assertions)]
    {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let background_panic = std::thread::current().id() != main_thread;
            restore_terminal();
            default_hook(info);
            if background_panic {
                std::process::abort();
            }
        }));
    }

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
        // Minimal delay for faster iteration, unless -d asks for a realistic
        // rate (e.g. to measure snapshot lag at a normal cadence).
        if args.delay.is_none() {
            app.config.refresh_rate_ms = 10;
        }
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
        &data_rx,
        &collector,
    );
    if let Some(stats) = bench_stats.as_mut() {
        stats.superseded_snapshots = data_rx.superseded_count();
    }

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
    data_rx: &data::SnapshotReceiver,
    collector: &data::DataCollector,
) -> Result<(), Box<dyn std::error::Error>> {
    // Wait on console input and a "snapshot published" event together, so a
    // snapshot is picked up with a thread wake instead of on the next timer
    // tick. The loop has no clock of its own to drift against the collector's
    // (issue #98).
    let waiter = event_wait::EventWait::new()?;
    data_rx.notify_on_publish({
        let signal = waiter.signal_handle();
        move || signal.raise()
    });
    // Consecutive input wakes that yielded no crossterm event (see below).
    let mut empty_input_wakes = 0u32;

    // Housekeeping cadence (process-info I/O refresh, config flush).
    let mut last_tick = Instant::now();
    let mut last_enrichment_bits = u8::MAX; // force the first store
    let mut last_collect_bits = u8::MAX; // force the first store
    let mut last_paused = app.paused;
    let mut last_rate_ms = app.config.refresh_rate_ms;

    // Paint the initial state before blocking on input: the loop below draws
    // only after handling events, so without this the first frame could wait
    // a full tick.
    let mut needs_redraw = {
        let draw_start = Instant::now();
        terminal.draw(|f| ui::draw(f, app))?;
        if let Some(stats) = bench_stats.as_mut() {
            stats.record_draw(draw_start.elapsed(), terminal.last_frame());
        }
        false
    };

    // Loop order: input → update-check → snapshot → deferred flush → draw.
    // Input is handled against the frame the user can see (drawn last
    // iteration); a keypress and a snapshot arriving in the same iteration
    // collapse into ONE deferred update pass and one draw instead of two.
    loop {
        let now = Instant::now();

        // Push collector-facing state only when it changed. A change the UI
        // is now waiting on (metadata it lacks, resuming from pause) wakes the
        // collector so it collects immediately instead of at its next tick.
        let mut wake_collector = false;
        if app.config.refresh_rate_ms != last_rate_ms {
            last_rate_ms = app.config.refresh_rate_ms;
            collector
                .tick_rate_ms
                .store(last_rate_ms, Ordering::Relaxed);
            wake_collector = true; // re-derive its schedule from now
        }
        if app.paused != last_paused {
            last_paused = app.paused;
            collector.paused.store(app.paused, Ordering::Relaxed);
            // A collector that skipped a tick while paused collects now.
            wake_collector |= !app.paused;
        }
        // Requirements only change with config/dialog state; skip the atomic
        // store when the bits are unchanged.
        let requirements = app.canonical_enrichment_requirements();
        let enrichment_bits = requirements.bits();
        if enrichment_bits != last_enrichment_bits {
            last_enrichment_bits = enrichment_bits;
            collector.set_enrichment_requirements(requirements);
            wake_collector |= !app.has_enrichment_for(requirements);
        }
        if wake_collector {
            collector.wake();
        }

        // Same change-detection for the collection gates (which subsystems
        // refresh() collects, mirroring meter visibility).
        let collect_bits = app.canonical_collect_requirements();
        if collect_bits != last_collect_bits {
            last_collect_bits = collect_bits;
            system::set_collect_gates(collect_bits);
        }

        // Read tick rate from app.config so it updates dynamically
        let tick_rate = Duration::from_millis(app.config.refresh_rate_ms);

        // Input crossterm already buffered comes first: our wait watches only
        // the console buffer, so an event sitting in crossterm's queue would
        // otherwise wait behind it. Then block until input, a published
        // snapshot, or the housekeeping tick.
        let mut input_ready = event::poll(Duration::ZERO)?;
        // A redraw already owed (deferred enrichment results) goes out now.
        if !input_ready && !needs_redraw {
            let housekeeping_left =
                tick_rate.saturating_sub(now.saturating_duration_since(last_tick));
            let woke_for_input = waiter.wait(housekeeping_left)? == event_wait::Wake::Input;
            if woke_for_input {
                input_ready = event::poll(Duration::ZERO)?;
            }
            // An input wake with no crossterm event means it consumed a record
            // it doesn't surface (e.g. a menu event). Back off if that repeats,
            // in case the console handle stays signaled with nothing to read.
            empty_input_wakes = if woke_for_input && !input_ready {
                empty_input_wakes + 1
            } else {
                0
            };
            if empty_input_wakes >= 3 {
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        // Handle input against the displayed frame before applying another
        // collector snapshot below. Process action keys must capture the
        // identity the user sees at the selected row.
        if input_ready {
            match event::read()? {
                Event::Key(key) => {
                    // Provably inert events (releases/repeats of mapped keys,
                    // bare modifier presses) cannot change state; skip the
                    // handler call and the redraw they would otherwise force
                    // (issue #83).
                    if !input::is_noop_key_event(app, &key) {
                        if input::handle_key_event(app, key) {
                            return Ok(());
                        }
                        needs_redraw = true;
                    }
                }
                Event::Mouse(mouse) => {
                    // Mouse moves and button releases are ignored by the
                    // handler; redrawing for each one burns a full frame per
                    // pointer movement (issue #83).
                    if !input::is_noop_mouse_event(app, &mouse) {
                        if input::handle_mouse_event(app, mouse) {
                            return Ok(());
                        }
                        needs_redraw = true;
                    }
                }
                Event::Resize(_, _) => {
                    terminal.refresh_size()?;
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
        let mut applied_published_at = None;
        {
            if let Ok(snapshot) = data_rx.try_recv() {
                if let Some(stats) = bench_stats.as_mut() {
                    stats.record_refresh(snapshot.refresh_duration);
                    stats.record_snapshot_lag(snapshot.published_at.elapsed());
                }
                applied_published_at = Some(snapshot.published_at);
                // The replaced process list goes back to the collector.
                let old = app.apply_snapshot(snapshot);
                let _ = collector.recycle_tx.send(old);
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

        // Flush deferred process list update once, after any snapshot above,
        // so a keypress + snapshot in the same iteration cost one pass.
        if app.needs_process_update {
            app.update_displayed_processes();
            app.needs_process_update = false;
            needs_redraw = true;
        }

        // Ctrl+L: clear the terminal and repaint every cell, repairing a
        // console garbled by other writers (issue #100).
        if app.full_redraw_requested {
            app.full_redraw_requested = false;
            terminal.clear()?;
            needs_redraw = true;
        }

        // Draw UI only when needed (state changed)
        if needs_redraw {
            // Rows scrolled into view since the last list update get their
            // metadata before being painted (issue #99).
            app.enrich_viewport();
            let draw_start = Instant::now();
            terminal.draw(|f| ui::draw(f, app))?;
            if let Some(stats) = bench_stats.as_mut() {
                stats.record_draw(draw_start.elapsed(), terminal.last_frame());
                if let Some(published_at) = applied_published_at {
                    stats.record_frame_latency(published_at.elapsed());
                }
            }
            needs_redraw = false;

            // Metadata queries for rows new to the view run after the frame
            // is out, then the frame is redrawn with their results.
            if app.run_deferred_enrichment() {
                needs_redraw = true;
            }
            // The console size is re-read here, off the snapshot-to-frame
            // path, instead of before each draw. Resize events need not
            // arrive (no window input without mouse capture), so a change
            // noticed here redraws right away.
            if terminal.refresh_size()? {
                needs_redraw = true;
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

    #[test]
    fn percentile_is_nearest_rank() {
        let ms = |n| Duration::from_millis(n);
        let samples: Vec<Duration> = (1..=100).rev().map(ms).collect();
        assert_eq!(percentile(&samples, 50), ms(50));
        assert_eq!(percentile(&samples, 99), ms(99));
        assert_eq!(percentile(&samples, 100), ms(100));
        assert_eq!(percentile(&[ms(7)], 99), ms(7));
        assert_eq!(percentile(&[], 50), Duration::ZERO);
    }
}
