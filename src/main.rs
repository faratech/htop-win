mod app;
mod config;
mod input;
mod system;
mod ui;

use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::App;
use config::Config;

/// htop-win: Interactive process viewer for Windows
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Refresh rate in milliseconds (default: 1000)
    #[arg(short = 'd', long = "delay", value_name = "MS")]
    delay: Option<u64>,

    /// Show only processes owned by the specified user
    #[arg(short, long, value_name = "USER")]
    user: Option<String>,

    /// Start in tree view mode
    #[arg(short, long)]
    tree: bool,

    /// Initial sort column (pid, cpu, mem, time, command)
    #[arg(short, long, value_name = "COLUMN")]
    sort: Option<String>,

    /// Disable mouse support
    #[arg(long = "no-mouse")]
    no_mouse: bool,

    /// Use monochrome (no colors)
    #[arg(long = "no-color")]
    no_color: bool,

    /// Show only specific PIDs (comma-separated)
    #[arg(short = 'p', long = "pid", value_name = "PID", value_delimiter = ',')]
    pids: Option<Vec<u32>>,

    /// Initial filter string
    #[arg(short = 'F', long = "filter", value_name = "FILTER")]
    filter: Option<String>,

    /// Exit after N updates
    #[arg(short = 'n', long = "max-iterations", value_name = "N")]
    max_iterations: Option<u64>,

    /// Hide header meters
    #[arg(long = "no-meters")]
    no_meters: bool,

    /// Readonly mode (disable kill/nice)
    #[arg(long = "readonly")]
    readonly: bool,

    /// Highlight process changes with delay in seconds
    #[arg(short = 'H', long = "highlight-changes", value_name = "DELAY")]
    highlight_changes: Option<u64>,
}

fn main() -> Result<()> {
    let args = Args::parse();

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
) -> Result<()> {
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
