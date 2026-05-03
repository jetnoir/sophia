use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use dns_lookup::lookup_host;
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect, Alignment},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph, Clear, List, ListItem},
    Frame, Terminal,
};
use std::{
    io,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use surge_ping::{Client, Config, ICMP, IcmpPacket, PingIdentifier, PingSequence};

#[derive(Parser, Debug)]
#[command(author, version, about = "SOPHIA: A modern, blazingly fast TUI ping tool for macOS.", long_about = "Copyright (c) 2026 Stuart Thomas. MIT for non-commercial use.")]
struct Args {
    /// Target hostname or IP address
    target: String,

    /// Interval between pings in milliseconds
    #[arg(short, long, default_value_t = 1000)]
    interval: u64,

    /// Timeout for each ping in milliseconds
    #[arg(short, long, default_value_t = 1000)]
    timeout: u64,
}

#[derive(Clone)]
struct PingResult {
    sequence: u16,
    rtt: Option<f64>,
    timestamp: String,
}

struct PingStats {
    current: Option<f64>,
    min: f64,
    max: f64,
    avg: f64,
    total_sent: u32,
    total_received: u32,
    jitter: f64,
    last_rtt: Option<f64>,
    history: Vec<(f64, f64)>, // (time, rtt)
    recent_pings: Vec<PingResult>,
}

impl PingStats {
    fn new() -> Self {
        Self {
            current: None,
            min: f64::MAX,
            max: 0.0,
            avg: 0.0,
            total_sent: 0,
            total_received: 0,
            jitter: 0.0,
            last_rtt: None,
            history: Vec::with_capacity(1000),
            recent_pings: Vec::with_capacity(100),
        }
    }

    fn update(&mut self, rtt: Option<Duration>, time: f64, sequence: u16) {
        self.total_sent += 1;
        let rtt_ms = rtt.map(|d| d.as_secs_f64() * 1000.0);
        
        self.recent_pings.push(PingResult {
            sequence,
            rtt: rtt_ms,
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
        });
        if self.recent_pings.len() > 100 {
            self.recent_pings.remove(0);
        }

        if let Some(ms) = rtt_ms {
            self.current = Some(ms);
            self.total_received += 1;
            
            if ms < self.min {
                self.min = ms;
            }
            if ms > self.max {
                self.max = ms;
            }
            
            self.avg = self.avg + (ms - self.avg) / self.total_received as f64;
            
            if let Some(last) = self.last_rtt {
                let diff = (ms - last).abs();
                self.jitter = self.jitter + (diff - self.jitter) / 16.0;
            }
            
            self.last_rtt = Some(ms);
            self.history.push((time, ms));
        } else {
            self.current = None;
        }

        if self.history.len() > 1000 {
            self.history.remove(0);
        }
    }

    fn loss_pct(&self) -> f64 {
        if self.total_sent == 0 {
            return 0.0;
        }
        (self.total_sent - self.total_received) as f64 / self.total_sent as f64 * 100.0
    }
}

#[derive(PartialEq)]
enum ViewMode {
    Dashboard,
    ChartOnly,
    Log,
    Stats,
}

struct App {
    target_name: String,
    target_ip: IpAddr,
    stats: Arc<Mutex<PingStats>>,
    view_mode: ViewMode,
    show_help: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let ips = lookup_host(&args.target)
        .with_context(|| format!("Failed to resolve host: {}", args.target))?;
    let target_ip = ips.first()
        .ok_or_else(|| anyhow::anyhow!("No IP address found for {}", args.target))?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let start_time = Instant::now();
    let stats = Arc::new(Mutex::new(PingStats::new()));
    let app = App {
        target_name: args.target.clone(),
        target_ip: *target_ip,
        stats: stats.clone(),
        view_mode: ViewMode::Dashboard,
        show_help: false,
    };

    let stats_clone = stats.clone();
    let target_ip_clone = *target_ip;
    let interval = Duration::from_millis(args.interval);
    let timeout = Duration::from_millis(args.timeout);

    tokio::spawn(async move {
        let mut config = Config::default();
        if target_ip_clone.is_ipv6() {
            config.kind = ICMP::V6;
        } else {
            config.kind = ICMP::V4;
        }

        let client = Client::new(&config).expect("Failed to create ICMP client");
        let mut pinger = client.pinger(target_ip_clone, PingIdentifier(111)).await;
        pinger.timeout(timeout);
        
        let mut sequence: u16 = 0;
        let payload = [0u8; 56];
        
        loop {
            let loop_start = Instant::now();
            let result = pinger.ping(PingSequence(sequence), &payload).await;
            let elapsed_since_start = start_time.elapsed().as_secs_f64();
            
            {
                if let Ok(mut stats_lock) = stats_clone.lock() {
                    match result {
                        Ok((IcmpPacket::V4(_), rtt)) | Ok((IcmpPacket::V6(_), rtt)) => {
                            stats_lock.update(Some(rtt), elapsed_since_start, sequence);
                        }
                        _ => {
                            stats_lock.update(None, elapsed_since_start, sequence);
                        }
                    }
                }
            }

            sequence = sequence.wrapping_add(1);
            let ping_elapsed = loop_start.elapsed();
            if ping_elapsed < interval {
                tokio::time::sleep(interval - ping_elapsed).await;
            }
        }
    });

    let res = run_app(&mut terminal, app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("\n[!] Application Error: {}", err);
    }

    Ok(())
}

async fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, &app))?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    return Ok(());
                }
                if key.code == KeyCode::Char('c') && key.modifiers.contains(event::KeyModifiers::CONTROL) {
                    return Ok(());
                }
                
                match key.code {
                    KeyCode::F(1) => app.show_help = !app.show_help,
                    KeyCode::Char('1') => app.view_mode = ViewMode::Dashboard,
                    KeyCode::Char('2') => app.view_mode = ViewMode::ChartOnly,
                    KeyCode::Char('3') => app.view_mode = ViewMode::Log,
                    KeyCode::Char('4') => app.view_mode = ViewMode::Stats,
                    KeyCode::Esc => app.show_help = false,
                    _ => {}
                }
            }
        }
    }
}

fn ui(f: &mut Frame, app: &App) {
    let area = f.size();
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Main Content
            Constraint::Length(1), // Footer hint
        ])
        .split(area);

    // Header
    let header_text = format!(
        " TARGET: {} ({}) | 'q' TO QUIT | F1 FOR HELP",
        app.target_name,
        app.target_ip,
    );
    let header = Paragraph::new(header_text)
        .block(Block::default().borders(Borders::ALL).title(" SOPHIA PING "))
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
    f.render_widget(header, main_chunks[0]);

    // Footer
    let footer = Paragraph::new(" [1] Dashboard  [2] Chart  [3] Log  [4] Stats  | F1 Help ")
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(footer, main_chunks[2]);

    // Main Content based on ViewMode
    match app.view_mode {
        ViewMode::Dashboard => render_dashboard(f, main_chunks[1], app),
        ViewMode::ChartOnly => render_chart_only(f, main_chunks[1], app),
        ViewMode::Log => render_log(f, main_chunks[1], app),
        ViewMode::Stats => render_stats_only(f, main_chunks[1], app),
    }

    if app.show_help {
        render_help_popup(f);
    }
}

fn render_dashboard(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),
            Constraint::Length(6),
        ])
        .split(area);

    render_latency_chart(f, chunks[0], app);
    render_stats_grid(f, chunks[1], app);
}

fn render_chart_only(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(3), // Small stats bar
        ])
        .split(area);

    render_latency_chart(f, chunks[0], app);
    
    let stats = app.stats.lock().unwrap();
    let cur_val = stats.current.map(|c| format!("{:.1}ms", c)).unwrap_or_else(|| "LOST".to_string());
    let bar_text = format!(" CURRENT: {} | MIN: {:.1}ms | MAX: {:.1}ms | AVG: {:.1}ms | LOSS: {:.1}% ", 
        cur_val, stats.min, stats.max, stats.avg, stats.loss_pct());
    let bar = Paragraph::new(bar_text)
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Yellow));
    f.render_widget(bar, chunks[1]);
}

fn render_log(f: &mut Frame, area: Rect, app: &App) {
    let stats = app.stats.lock().unwrap();
    let items: Vec<ListItem> = stats.recent_pings.iter().rev().map(|p| {
        let content = if let Some(rtt) = p.rtt {
            Line::from(vec![
                Span::styled(format!("[{}] ", p.timestamp), Style::default().fg(Color::DarkGray)),
                Span::raw(format!("64 bytes from {}: icmp_seq={} time=", app.target_ip, p.sequence)),
                Span::styled(format!("{:.2} ms", rtt), Style::default().fg(if rtt < 50.0 { Color::Green } else if rtt < 150.0 { Color::Yellow } else { Color::Red })),
            ])
        } else {
            Line::from(vec![
                Span::styled(format!("[{}] ", p.timestamp), Style::default().fg(Color::DarkGray)),
                Span::styled(format!("Request timeout for icmp_seq {}", p.sequence), Style::default().fg(Color::Red)),
            ])
        };
        ListItem::new(content)
    }).collect();

    let list = List::new(items)
        .block(Block::default().title(" PING LOG ").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::ITALIC));
    f.render_widget(list, area);
}

fn render_stats_only(f: &mut Frame, area: Rect, app: &App) {
    let stats = app.stats.lock().unwrap();
    let text = vec![
        Line::from(vec![Span::styled("--- STATISTICS ---", Style::default().add_modifier(Modifier::BOLD))]),
        Line::from(""),
        Line::from(vec![Span::raw("Current Latency: "), Span::styled(format!("{:.2} ms", stats.current.unwrap_or(0.0)), Style::default().fg(Color::Cyan))]),
        Line::from(vec![Span::raw("Minimum Latency: "), Span::styled(format!("{:.2} ms", stats.min), Style::default().fg(Color::Green))]),
        Line::from(vec![Span::raw("Maximum Latency: "), Span::styled(format!("{:.2} ms", stats.max), Style::default().fg(Color::Red))]),
        Line::from(vec![Span::raw("Average Latency: "), Span::styled(format!("{:.2} ms", stats.avg), Style::default().fg(Color::Yellow))]),
        Line::from(vec![Span::raw("Jitter:          "), Span::styled(format!("{:.2} ms", stats.jitter), Style::default().fg(Color::Magenta))]),
        Line::from(""),
        Line::from(vec![Span::raw("Packets Sent:    "), Span::raw(stats.total_sent.to_string())]),
        Line::from(vec![Span::raw("Packets Recv:    "), Span::raw(stats.total_received.to_string())]),
        Line::from(vec![Span::raw("Packet Loss:     "), Span::styled(format!("{:.2}%", stats.loss_pct()), Style::default().fg(if stats.loss_pct() > 0.0 { Color::Red } else { Color::Green }))]),
    ];

    let p = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title(" DETAILED STATS "))
        .alignment(Alignment::Left);
    f.render_widget(p, area);
}

fn render_latency_chart(f: &mut Frame, area: Rect, app: &App) {
    let stats = app.stats.lock().unwrap();
    let history = &stats.history;
    let max_time = history.last().map(|(t, _)| *t).unwrap_or(0.0);
    let min_time = history.first().map(|(t, _)| *t).unwrap_or(0.0);
    
    let datasets = vec![Dataset::default()
        .name("Latency")
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(match stats.current {
            Some(c) if c < 50.0 => Color::Green,
            Some(c) if c < 150.0 => Color::Yellow,
            Some(_) => Color::Red,
            None => Color::DarkGray,
        }))
        .data(history)];

    let chart = Chart::new(datasets)
        .block(Block::default().title(format!(" LATENCY HISTORY (MAX: {:.1}ms) ", stats.max)).borders(Borders::ALL))
        .x_axis(Axis::default().title("Seconds").style(Style::default().fg(Color::Gray)).bounds([min_time, max_time]))
        .y_axis(Axis::default().title("ms").style(Style::default().fg(Color::Gray)).bounds([0.0, (stats.max * 1.1).max(50.0)])
            .labels(vec![Span::raw("0"), Span::raw(format!("{:.0}", stats.max / 2.0)), Span::raw(format!("{:.0}", stats.max))]));
    f.render_widget(chart, area);
}

fn render_stats_grid(f: &mut Frame, area: Rect, app: &App) {
    let stats = app.stats.lock().unwrap();
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(16),
            Constraint::Percentage(16),
            Constraint::Percentage(16),
            Constraint::Percentage(16),
            Constraint::Percentage(16),
            Constraint::Percentage(20),
        ])
        .split(area);

    let render_stat = |f: &mut Frame, area: Rect, title: &str, val: String, color: Color| {
        let p = Paragraph::new(val)
            .block(Block::default().borders(Borders::ALL).title(title))
            .alignment(Alignment::Center)
            .style(Style::default().fg(color).add_modifier(Modifier::BOLD));
        f.render_widget(p, area);
    };

    let cur_val = stats.current.map(|c| format!("{:.1}ms", c)).unwrap_or_else(|| "LOST".to_string());
    let cur_col = if stats.current.is_none() { Color::Red } else { Color::Cyan };

    render_stat(f, chunks[0], " CURRENT ", cur_val, cur_col);
    render_stat(f, chunks[1], " MIN ", format!("{:.1}ms", if stats.min == f64::MAX { 0.0 } else { stats.min }), Color::Green);
    render_stat(f, chunks[2], " MAX ", format!("{:.1}ms", stats.max), Color::Red);
    render_stat(f, chunks[3], " AVG ", format!("{:.1}ms", stats.avg), Color::Yellow);
    render_stat(f, chunks[4], " JITTER ", format!("{:.1}ms", stats.jitter), Color::Magenta);
    render_stat(f, chunks[5], " LOSS ", format!("{:.1}%", stats.loss_pct()), if stats.loss_pct() > 0.0 { Color::Red } else { Color::Green });
}

fn render_help_popup(f: &mut Frame) {
    let area = centered_rect(60, 40, f.size());
    f.render_widget(Clear, area); // Clear the background
    let help_text = vec![
        Line::from(vec![Span::styled("SOPHIA Help Menu", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))]),
        Line::from(""),
        Line::from(vec![Span::styled("View Modes:", Style::default().add_modifier(Modifier::UNDERLINED))]),
        Line::from(" [1] Dashboard - Default view with chart and stats"),
        Line::from(" [2] Chart     - Maximized chart with stats bar"),
        Line::from(" [3] Log       - Scrolling list of ping results"),
        Line::from(" [4] Stats     - Detailed statistics summary"),
        Line::from(""),
        Line::from(vec![Span::styled("Controls:", Style::default().add_modifier(Modifier::UNDERLINED))]),
        Line::from(" F1         - Toggle this help menu"),
        Line::from(" q          - Quit the application"),
        Line::from(" Ctrl+C     - Force quit"),
        Line::from(" Esc        - Close help menu"),
        Line::from(""),
        Line::from(vec![Span::styled("Built by Stuart Thomas", Style::default().fg(Color::DarkGray))]),
    ];

    let help = Paragraph::new(help_text)
        .block(Block::default().title(" HELP ").borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan)))
        .alignment(Alignment::Left);
    f.render_widget(help, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
