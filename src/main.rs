mod app;
mod config;
mod input;
mod system;
mod ui;

use std::io;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

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
    println!("      --readonly               Disable kill/nice operations");
    println!("  -H, --highlight-changes <S>  Highlight process changes (seconds)");
    println!("  -h, --help                   Print help");
    println!("  -V, --version                Print version");
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

    // Apply PID filter from CLI
    if let Some(ref pids) = args.pids {
        app.pid_filter = Some(pids.clone());
    }

    // Apply max iterations from CLI
    if let Some(n) = args.max_iterations {
        app.max_iterations = Some(n);
    }

    // Apply no-meters from CLI
    if args.no_meters {
        app.show_header = false;
    }

    // Initial system refresh
    app.refresh_system();

    // Run the main loop
    let result = run_app(&mut terminal, &mut app, &config);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = result {
        eprintln!("Error: {err:?}");
    }

    Ok(())
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    _config: &Config,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_tick = Instant::now();

    loop {
        // Read tick rate from app.config so it updates dynamically
        let tick_rate = Duration::from_millis(app.config.refresh_rate_ms);

        // Draw UI
        terminal.draw(|f| ui::draw(f, app))?;

        // Handle events with timeout
        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    if input::handle_key_event(app, key) {
                        return Ok(());
                    }
                }
                Event::Mouse(mouse) => {
                    input::handle_mouse_event(app, mouse);
                }
                Event::Resize(_, _) => {
                    // Terminal will handle resize automatically
                }
                _ => {}
            }
        }

        // Refresh system data at tick rate (unless paused)
        if last_tick.elapsed() >= tick_rate && !app.paused {
            app.refresh_system();
            app.iteration_count += 1;

            // Check if we've reached max iterations
            if let Some(max) = app.max_iterations {
                if app.iteration_count >= max {
                    return Ok(());
                }
            }
            last_tick = Instant::now();
        }
    }
}
