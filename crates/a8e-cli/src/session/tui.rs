//! Ratatui-based inline viewport widgets for enhanced CLI visuals.
//!
//! Uses Ratatui's inline viewport mode to render structured panels
//! (status, credits, greeting) without entering alternate screen,
//! preserving terminal scrollback history.

#[cfg(feature = "tui")]
mod inner {
    use crossterm::{
        terminal::{disable_raw_mode, enable_raw_mode},
        ExecutableCommand,
    };
    use ratatui::{
        backend::CrosstermBackend,
        layout::{Constraint, Direction, Layout, Rect},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Gauge, Paragraph, Row, Table},
        Terminal,
    };
    use std::io::{self, stdout, IsTerminal};

    // Articulate's hero glyph in the Paean Family is `∞`. The brand color
    // (magenta) is its individual mark; cyan acts as the supporting accent
    // shared with the rest of the family.
    const BRAND_COLOR: Color = Color::Magenta;
    const ACCENT_COLOR: Color = Color::Cyan;
    const SUCCESS_COLOR: Color = Color::Green;
    const WARNING_COLOR: Color = Color::Yellow;
    const ERROR_COLOR: Color = Color::Red;
    const DIM_COLOR: Color = Color::DarkGray;

    // Paean Family sigil — the trio of sibling product glyphs, rendered
    // dimly anywhere we want to nod to the ecosystem without competing
    // with Articulate's hero `∞`.
    const FAMILY_SIGIL: &str = "\u{2b2c} \u{00b7} \u{2229} \u{00b7} \u{221e}"; // ⌬ · ∩ · ∞
    const FAMILY_TAG: &str = "Paean Family";

    pub struct CreditsInfo {
        pub credits: i64,
        pub total_credits: i64,
        pub subscription_tier: String,
        pub next_recovery_at: Option<String>,
        pub can_recover: bool,
        pub recovery_interval_hours: i64,
        pub billing_period: Option<String>,
        pub subscription_end_date: Option<String>,
    }

    pub struct SessionStatusInfo {
        pub session_id: String,
        pub provider: String,
        pub model: String,
        pub mode: String,
        pub extensions: Vec<String>,
        pub total_tokens: Option<i32>,
        pub context_limit: usize,
        pub cwd: String,
    }

    fn render_inline<F>(height: u16, render_fn: F) -> io::Result<()>
    where
        F: FnOnce(&mut ratatui::Frame, Rect),
    {
        if !stdout().is_terminal() {
            return Ok(());
        }

        let mut stdout = stdout();
        enable_raw_mode()?;
        stdout.execute(crossterm::cursor::Hide)?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::with_options(
            backend,
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Inline(height),
            },
        )?;

        terminal.draw(|frame| {
            let area = frame.area();
            render_fn(frame, area);
        })?;

        let mut stdout = io::stdout();
        stdout.execute(crossterm::cursor::Show)?;
        disable_raw_mode()?;
        // Move cursor below the inline viewport
        println!();

        Ok(())
    }

    pub fn render_greeting(version: &str) -> io::Result<()> {
        // Reserve an extra row for the Paean Family lockup beneath the hint.
        render_inline(11, |frame, area| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(7),
                    Constraint::Length(2),
                    Constraint::Length(1),
                ])
                .split(area);

            let logo_lines = vec![
                Line::from(Span::styled(
                    "    _         _   _            _       _       ",
                    Style::default()
                        .fg(BRAND_COLOR)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    "   / \\   _ __| |_(_) ___ _   _| | __ _| |_ ___ ",
                    Style::default()
                        .fg(BRAND_COLOR)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    r"  / _ \ | '__| __| |/ __| | | | |/ _` | __/ _ \",
                    Style::default()
                        .fg(BRAND_COLOR)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    r" / ___ \| |  | |_| | (__| |_| | | (_| | ||  __/",
                    Style::default()
                        .fg(BRAND_COLOR)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    r"/_/   \_\_|   \__|_|\___|\__,_|_|\__,_|\__\___|",
                    Style::default()
                        .fg(BRAND_COLOR)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        " \u{221e} a8e ",
                        Style::default()
                            .fg(BRAND_COLOR)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("v{}", version),
                        Style::default()
                            .fg(ACCENT_COLOR)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        " \u{00b7} Speak freely, code locally.",
                        Style::default().fg(DIM_COLOR),
                    ),
                ]),
            ];

            let logo = Paragraph::new(logo_lines).block(Block::default().borders(Borders::NONE));
            frame.render_widget(logo, chunks[0]);

            let hint = Line::from(vec![
                Span::styled(
                    " \u{276f} ",
                    Style::default()
                        .fg(BRAND_COLOR)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Type a message to get started",
                    Style::default().fg(DIM_COLOR),
                ),
                Span::styled("  \u{00b7}  ", Style::default().fg(DIM_COLOR)),
                Span::styled(
                    "/help for commands",
                    Style::default().fg(ACCENT_COLOR),
                ),
            ]);

            let hint_widget = Paragraph::new(hint).block(Block::default().borders(Borders::NONE));
            frame.render_widget(hint_widget, chunks[1]);

            // Paean Family lockup — sibling-product nod, dimly rendered so
            // it acts as a watermark rather than competing with the logo.
            let family = Line::from(vec![
                Span::styled(
                    format!(" {}  ", FAMILY_TAG),
                    Style::default().fg(DIM_COLOR),
                ),
                Span::styled(FAMILY_SIGIL, Style::default().fg(ACCENT_COLOR)),
            ]);
            let family_widget =
                Paragraph::new(family).block(Block::default().borders(Borders::NONE));
            frame.render_widget(family_widget, chunks[2]);
        })
    }

    pub fn render_session_banner(
        status: &str,
        provider: &str,
        model: &str,
        session_id: Option<&str>,
        cwd: &str,
        auth_email: Option<&str>,
    ) -> io::Result<()> {
        let height = if auth_email.is_some() { 5 } else { 4 };
        render_inline(height, |frame, area| {
            let status_color = match status {
                "resuming" => WARNING_COLOR,
                "ephemeral" => BRAND_COLOR,
                _ => SUCCESS_COLOR,
            };

            let mut lines = vec![
                Line::from(vec![
                    Span::styled(" \u{25cf} ", Style::default().fg(status_color)),
                    Span::styled(
                        format!("{} ", status),
                        Style::default()
                            .fg(status_color)
                            .add_modifier(Modifier::DIM),
                    ),
                    Span::styled("\u{00b7} ", Style::default().fg(DIM_COLOR)),
                    Span::styled(format!("{} ", provider), Style::default().fg(DIM_COLOR)),
                    Span::styled(model, Style::default().fg(ACCENT_COLOR)),
                ]),
                Line::from(vec![
                    Span::styled("   ", Style::default()),
                    Span::styled(
                        session_id.unwrap_or("ephemeral"),
                        Style::default().fg(DIM_COLOR),
                    ),
                    Span::styled(" \u{00b7} ", Style::default().fg(DIM_COLOR)),
                    Span::styled(cwd, Style::default().fg(DIM_COLOR)),
                ]),
            ];

            if let Some(email) = auth_email {
                lines.push(Line::from(vec![
                    Span::styled("   ", Style::default()),
                    Span::styled("\u{1f513} ", Style::default().fg(SUCCESS_COLOR)),
                    Span::styled(email, Style::default().fg(DIM_COLOR)),
                ]));
            }

            let banner = Paragraph::new(lines).block(Block::default().borders(Borders::NONE));
            frame.render_widget(banner, area);
        })
    }

    pub fn render_credits_panel(info: &CreditsInfo) -> io::Result<()> {
        render_inline(10, |frame, area| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // title
                    Constraint::Length(1), // spacer
                    Constraint::Length(2), // gauge
                    Constraint::Length(1), // spacer
                    Constraint::Min(4),    // details
                ])
                .split(area);

            // Title
            let title = Line::from(vec![Span::styled(
                " Credits & Usage ",
                Style::default()
                    .fg(ACCENT_COLOR)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )]);
            frame.render_widget(Paragraph::new(title), chunks[0]);

            // Credits gauge
            let ratio = if info.total_credits > 0 {
                (info.credits as f64 / info.total_credits as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let gauge_color = if ratio > 0.5 {
                SUCCESS_COLOR
            } else if ratio > 0.15 {
                WARNING_COLOR
            } else {
                ERROR_COLOR
            };

            let gauge = Gauge::default()
                .block(Block::default().borders(Borders::NONE))
                .gauge_style(Style::default().fg(gauge_color))
                .ratio(ratio)
                .label(Span::styled(
                    format!(
                        "{} / {} credits  ({}%)",
                        info.credits,
                        info.total_credits,
                        (ratio * 100.0).round() as u32,
                    ),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ));
            frame.render_widget(gauge, chunks[2]);

            // Details table
            let mut rows = vec![
                Row::new(vec![
                    Span::styled("Tier", Style::default().fg(ACCENT_COLOR)),
                    Span::styled(&info.subscription_tier, Style::default().fg(DIM_COLOR)),
                ]),
                Row::new(vec![
                    Span::styled("Recovery", Style::default().fg(ACCENT_COLOR)),
                    Span::styled(
                        if info.can_recover {
                            "Available now".to_string()
                        } else if let Some(ref next) = info.next_recovery_at {
                            next.clone()
                        } else {
                            format!("Every {}h", info.recovery_interval_hours)
                        },
                        Style::default().fg(if info.can_recover {
                            SUCCESS_COLOR
                        } else {
                            DIM_COLOR
                        }),
                    ),
                ]),
            ];

            if let Some(ref billing) = info.billing_period {
                rows.push(Row::new(vec![
                    Span::styled("Billing", Style::default().fg(ACCENT_COLOR)),
                    Span::styled(billing, Style::default().fg(DIM_COLOR)),
                ]));
            }

            if let Some(ref end_date) = info.subscription_end_date {
                rows.push(Row::new(vec![
                    Span::styled("Ends", Style::default().fg(ACCENT_COLOR)),
                    Span::styled(end_date, Style::default().fg(DIM_COLOR)),
                ]));
            }

            let table = Table::new(rows, [Constraint::Length(16), Constraint::Min(20)])
                .block(Block::default().borders(Borders::NONE));
            frame.render_widget(table, chunks[4]);
        })
    }

    pub fn render_status_panel(
        info: &SessionStatusInfo,
        credits: Option<&CreditsInfo>,
    ) -> io::Result<()> {
        let has_credits = credits.is_some();
        let height = if has_credits { 16 } else { 10 };

        render_inline(height, |frame, area| {
            let main_chunks = if has_credits {
                Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(9),
                        Constraint::Length(1),
                        Constraint::Min(5),
                    ])
                    .split(area)
            } else {
                Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(9)])
                    .split(area)
            };

            // Session info
            let title = Line::from(Span::styled(
                " Session Status ",
                Style::default()
                    .fg(ACCENT_COLOR)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ));

            let context_pct = if info.context_limit > 0 {
                let total = info.total_tokens.unwrap_or(0) as usize;
                format!(
                    "{} / {} tokens ({}%)",
                    total,
                    format_tokens(info.context_limit),
                    ((total as f64 / info.context_limit as f64) * 100.0).round() as u32,
                )
            } else {
                format!("{} tokens", info.total_tokens.unwrap_or(0))
            };

            let ext_display = if info.extensions.is_empty() {
                "none".to_string()
            } else {
                info.extensions.join(", ")
            };

            let rows = vec![
                Row::new(vec![
                    Span::styled("Session", Style::default().fg(ACCENT_COLOR)),
                    Span::styled(&info.session_id, Style::default().fg(DIM_COLOR)),
                ]),
                Row::new(vec![
                    Span::styled("Working dir", Style::default().fg(ACCENT_COLOR)),
                    Span::styled(&info.cwd, Style::default().fg(DIM_COLOR)),
                ]),
                Row::new(vec![
                    Span::styled("Provider", Style::default().fg(ACCENT_COLOR)),
                    Span::styled(&info.provider, Style::default().fg(DIM_COLOR)),
                ]),
                Row::new(vec![
                    Span::styled("Model", Style::default().fg(ACCENT_COLOR)),
                    Span::styled(&info.model, Style::default().fg(DIM_COLOR)),
                ]),
                Row::new(vec![
                    Span::styled("Mode", Style::default().fg(ACCENT_COLOR)),
                    Span::styled(&info.mode, Style::default().fg(DIM_COLOR)),
                ]),
                Row::new(vec![
                    Span::styled("Context", Style::default().fg(ACCENT_COLOR)),
                    Span::styled(&context_pct, Style::default().fg(DIM_COLOR)),
                ]),
                Row::new(vec![
                    Span::styled("Extensions", Style::default().fg(ACCENT_COLOR)),
                    Span::styled(&ext_display, Style::default().fg(DIM_COLOR)),
                ]),
            ];

            let session_block = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(7),
                ])
                .split(main_chunks[0]);

            frame.render_widget(Paragraph::new(title), session_block[0]);

            let table = Table::new(rows, [Constraint::Length(16), Constraint::Min(30)])
                .block(Block::default().borders(Borders::NONE));
            frame.render_widget(table, session_block[2]);

            // Credits section
            if let Some(credits) = credits {
                let ratio = if credits.total_credits > 0 {
                    (credits.credits as f64 / credits.total_credits as f64).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let gauge_color = if ratio > 0.5 {
                    SUCCESS_COLOR
                } else if ratio > 0.15 {
                    WARNING_COLOR
                } else {
                    ERROR_COLOR
                };

                let credits_title = Line::from(Span::styled(
                    " Credits ",
                    Style::default()
                        .fg(ACCENT_COLOR)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                ));
                frame.render_widget(Paragraph::new(credits_title), main_chunks[1]);

                let credits_area = main_chunks[2];
                let credits_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(2),
                    ])
                    .split(credits_area);

                let gauge = Gauge::default()
                    .block(Block::default().borders(Borders::NONE))
                    .gauge_style(Style::default().fg(gauge_color))
                    .ratio(ratio)
                    .label(Span::styled(
                        format!(
                            "{} / {}  ({})  {}",
                            credits.credits,
                            credits.total_credits,
                            credits.subscription_tier,
                            if credits.can_recover {
                                "| Recovery available"
                            } else {
                                ""
                            },
                        ),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ));
                frame.render_widget(gauge, credits_chunks[1]);

                if ratio < 0.15 {
                    let warning = Line::from(vec![
                        Span::styled(
                            " Low credits! ",
                            Style::default()
                                .fg(ERROR_COLOR)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            "Visit one.paean.ai to upgrade or add credits",
                            Style::default().fg(WARNING_COLOR),
                        ),
                    ]);
                    frame.render_widget(Paragraph::new(warning), credits_chunks[2]);
                }
            }
        })
    }

    pub fn render_credits_warning(credits: i64, total: i64) -> io::Result<()> {
        let ratio = if total > 0 {
            credits as f64 / total as f64
        } else {
            1.0
        };
        if ratio >= 0.2 {
            return Ok(());
        }

        render_inline(2, |frame, area| {
            let warning = Line::from(vec![
                Span::styled(" \u{26a0} ", Style::default().fg(WARNING_COLOR)),
                Span::styled(
                    format!("Credits low: {}/{}", credits, total),
                    Style::default()
                        .fg(WARNING_COLOR)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  \u{2192}  ", Style::default().fg(DIM_COLOR)),
                Span::styled(
                    "one.paean.ai",
                    Style::default()
                        .fg(ACCENT_COLOR)
                        .add_modifier(Modifier::UNDERLINED),
                ),
                Span::styled(" to upgrade", Style::default().fg(DIM_COLOR)),
            ]);
            frame.render_widget(
                Paragraph::new(warning).block(Block::default().borders(Borders::NONE)),
                area,
            );
        })
    }

    fn format_tokens(n: usize) -> String {
        if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1_000_000.0)
        } else if n >= 1_000 {
            format!("{}k", n / 1_000)
        } else {
            n.to_string()
        }
    }
}

#[cfg(feature = "tui")]
pub use inner::*;

// Plain-text fallback when tui feature is disabled
#[cfg(not(feature = "tui"))]
pub struct CreditsInfo {
    pub credits: i64,
    pub total_credits: i64,
    pub subscription_tier: String,
    pub next_recovery_at: Option<String>,
    pub can_recover: bool,
    pub recovery_interval_hours: i64,
    pub billing_period: Option<String>,
    pub subscription_end_date: Option<String>,
}

#[cfg(not(feature = "tui"))]
pub struct SessionStatusInfo {
    pub session_id: String,
    pub provider: String,
    pub model: String,
    pub mode: String,
    pub extensions: Vec<String>,
    pub total_tokens: Option<i32>,
    pub context_limit: usize,
    pub cwd: String,
}

#[cfg(not(feature = "tui"))]
pub fn render_greeting(_version: &str) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(feature = "tui"))]
pub fn render_session_banner(
    _status: &str,
    _provider: &str,
    _model: &str,
    _session_id: Option<&str>,
    _cwd: &str,
    _auth_email: Option<&str>,
) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(feature = "tui"))]
pub fn render_credits_panel(_info: &CreditsInfo) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(feature = "tui"))]
pub fn render_status_panel(
    _info: &SessionStatusInfo,
    _credits: Option<&CreditsInfo>,
) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(feature = "tui"))]
pub fn render_credits_warning(_credits: i64, _total: i64) -> std::io::Result<()> {
    Ok(())
}
