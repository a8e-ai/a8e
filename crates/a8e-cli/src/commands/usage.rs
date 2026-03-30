use a8e_core::paean_api;
use anyhow::Result;
use console::style;

use crate::session::tui;

pub async fn handle_usage() -> Result<()> {
    if !paean_api::is_authenticated() {
        println!(
            "\n  {} Not authenticated with Paean AI.",
            style("\u{26a0}").yellow()
        );
        println!(
            "  {}",
            style("Run `a8e login` or set PAEAN_AI_API_KEY to view credits.").dim()
        );
        return Ok(());
    }

    println!(
        "\n  {} Fetching credits status...",
        style("\u{21bb}").cyan()
    );

    match paean_api::get_credits_status().await {
        Ok(status) => {
            let credits_info = tui::CreditsInfo {
                credits: status.credits,
                total_credits: status.total_credits,
                subscription_tier: status.subscription_tier.clone(),
                next_recovery_at: status.next_recovery_at.clone(),
                can_recover: status.can_recover,
                recovery_interval_hours: status.recovery_interval_hours,
                billing_period: status.billing_period.clone(),
                subscription_end_date: status.subscription_end_date.clone(),
            };

            if tui::render_credits_panel(&credits_info).is_err() {
                render_credits_plain(&status);
            }

            if status.credits < 20 {
                println!(
                    "\n  {} {}",
                    style("\u{26a0}").yellow().bold(),
                    style(format!(
                        "Credits are low ({}). Visit one.paean.ai to upgrade or add credits.",
                        status.credits
                    ))
                    .yellow()
                );
            }
        }
        Err(e) => {
            println!(
                "\n  {} Failed to fetch credits: {}",
                style("\u{2718}").red(),
                style(e.to_string()).red()
            );
        }
    }

    Ok(())
}

pub async fn handle_claim() -> Result<()> {
    if !paean_api::is_authenticated() {
        println!("\n  {} Not authenticated.", style("\u{26a0}").yellow());
        return Ok(());
    }

    println!(
        "\n  {} Claiming credits recovery...",
        style("\u{21bb}").cyan()
    );

    match paean_api::claim_credits().await {
        Ok(result) => {
            if result.success {
                let msg = result
                    .message
                    .unwrap_or_else(|| "Credits recovered".to_string());
                println!(
                    "\n  {} {}",
                    style("\u{2713}").green().bold(),
                    style(&msg).green()
                );
                if let Some(data) = result.data {
                    if let (Some(credits), Some(total)) = (data.credits, data.total_credits) {
                        println!(
                            "  {} {} / {}",
                            style("Credits:").dim(),
                            style(credits).cyan(),
                            style(total).dim()
                        );
                    }
                    if let Some(next) = data.next_recovery_at {
                        println!("  {} {}", style("Next recovery:").dim(), style(&next).dim());
                    }
                }
            } else {
                let msg = result
                    .message
                    .unwrap_or_else(|| "Not eligible for recovery yet".to_string());
                println!(
                    "\n  {} {}",
                    style("\u{26a0}").yellow(),
                    style(&msg).yellow()
                );
            }
        }
        Err(e) => {
            println!(
                "\n  {} Failed to claim credits: {}",
                style("\u{2718}").red(),
                style(e.to_string()).red()
            );
        }
    }

    Ok(())
}

fn render_credits_plain(status: &paean_api::CreditsStatus) {
    println!();
    println!("  {}", style("Credits Status").bold().underlined());
    println!();
    println!(
        "  {}  {} / {} credits",
        style(format!("{:<14}", "Balance")).cyan(),
        style(status.credits).bold(),
        style(status.total_credits).dim()
    );
    println!(
        "  {}  {}",
        style(format!("{:<14}", "Tier")).cyan(),
        style(&status.subscription_tier).dim()
    );
    if status.can_recover {
        println!(
            "  {}  {}",
            style(format!("{:<14}", "Recovery")).cyan(),
            style("Available now").green()
        );
    } else if let Some(ref next) = status.next_recovery_at {
        println!(
            "  {}  {}",
            style(format!("{:<14}", "Next Recovery")).cyan(),
            style(next).dim()
        );
    }
    println!(
        "  {}  {}h",
        style(format!("{:<14}", "Interval")).cyan(),
        style(status.recovery_interval_hours).dim()
    );
    if let Some(ref billing) = status.billing_period {
        println!(
            "  {}  {}",
            style(format!("{:<14}", "Billing")).cyan(),
            style(billing).dim()
        );
    }
    if let Some(ref end_date) = status.subscription_end_date {
        println!(
            "  {}  {}",
            style(format!("{:<14}", "Ends")).cyan(),
            style(end_date).dim()
        );
    }
    println!();
}

/// Render usage info inside an interactive session (called from /usage slash command)
pub async fn handle_usage_inline() -> Result<()> {
    handle_usage().await
}
