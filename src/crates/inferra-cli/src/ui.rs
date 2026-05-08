use std::io::{self, IsTerminal};

use clap::builder::styling::{AnsiColor, Effects, Styles};
use comfy_table::{presets::ASCII_MARKDOWN, ContentArrangement, Table};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::Value as JsonValue;

pub fn clap_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
        .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
        .literal(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
        .placeholder(AnsiColor::Magenta.on_default())
        .valid(AnsiColor::Green.on_default())
        .invalid(AnsiColor::Red.on_default().effects(Effects::BOLD))
        .error(AnsiColor::Red.on_default().effects(Effects::BOLD))
}

#[derive(Clone, Debug)]
pub struct TerminalUi {
    json: bool,
    interactive: bool,
}

impl TerminalUi {
    pub fn new(json: bool) -> Self {
        let interactive = !json && io::stdout().is_terminal() && io::stderr().is_terminal();
        Self { json, interactive }
    }

    pub fn is_json(&self) -> bool {
        self.json
    }

    pub fn is_interactive(&self) -> bool {
        self.interactive
    }

    pub fn print_json(&self, payload: &JsonValue) {
        println!(
            "{}",
            serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string())
        );
    }

    pub fn banner(&self, title: &str, subtitle: &str) {
        if self.json {
            return;
        }
        println!("{}", style(title).bold().cyan());
        if !subtitle.is_empty() {
            println!("{}", style(subtitle).dim());
        }
        println!();
    }

    pub fn section(&self, title: &str) {
        if self.json {
            return;
        }
        println!("{}", style(title).bold().underlined());
    }

    pub fn paragraph(&self, text: &str) {
        if self.json {
            return;
        }
        println!("{text}");
    }

    pub fn info(&self, text: &str) {
        if self.json {
            return;
        }
        println!("{} {}", style("INFO").blue().bold(), text);
    }

    pub fn success(&self, text: &str) {
        if self.json {
            return;
        }
        println!("{} {}", style("OK").green().bold(), text);
    }

    pub fn warning(&self, text: &str) {
        if self.json {
            return;
        }
        println!("{} {}", style("WARN").yellow().bold(), text);
    }

    pub fn bullets<I>(&self, items: I)
    where
        I: IntoIterator<Item = String>,
    {
        if self.json {
            return;
        }
        for item in items {
            println!("  {} {}", style("-").cyan().bold(), item);
        }
    }

    pub fn kv_table<I>(&self, rows: I)
    where
        I: IntoIterator<Item = (&'static str, String)>,
    {
        if self.json {
            return;
        }
        let mut table = Table::new();
        table
            .load_preset(ASCII_MARKDOWN)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(vec![
                style("Field").bold().to_string(),
                style("Value").bold().to_string(),
            ]);
        for (label, value) in rows {
            table.add_row(vec![style(label).cyan().to_string(), value]);
        }
        println!("{table}");
    }

    pub fn table<I>(&self, headers: &[&str], rows: I)
    where
        I: IntoIterator<Item = Vec<String>>,
    {
        if self.json {
            return;
        }
        let mut table = Table::new();
        table
            .load_preset(ASCII_MARKDOWN)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(
                headers
                    .iter()
                    .map(|header| style(header).bold().to_string())
                    .collect::<Vec<_>>(),
            );
        for row in rows {
            table.add_row(row);
        }
        println!("{table}");
    }

    pub fn spinner(&self, message: impl Into<String>) -> SpinnerHandle {
        let message = message.into();
        if !self.interactive {
            return SpinnerHandle { progress: None };
        }
        let progress = ProgressBar::new_spinner();
        progress.set_style(
            ProgressStyle::with_template("{spinner} {msg}")
                .unwrap_or_else(|_| ProgressStyle::default_spinner())
                .tick_strings(&["-", "\\", "|", "/"]),
        );
        progress.enable_steady_tick(std::time::Duration::from_millis(80));
        progress.set_message(message);
        SpinnerHandle {
            progress: Some(progress),
        }
    }
}

pub struct SpinnerHandle {
    progress: Option<ProgressBar>,
}

impl SpinnerHandle {
    pub fn finish(self, message: &str) {
        if let Some(progress) = self.progress {
            progress.finish_and_clear();
            println!("{} {}", style("OK").green().bold(), message);
        }
    }
}
