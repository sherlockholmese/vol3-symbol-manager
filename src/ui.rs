//! Terminal presentation roles. Commands use these instead of raw colors so
//! every surface shares one vocabulary: accent marks installable objects,
//! muted marks structure and hints, success and danger report outcomes.

use colored::{ColoredString, Colorize};
use indicatif::{ProgressBar, ProgressStyle};
use std::fmt;
use std::time::Duration;

pub fn accent(s: &str) -> ColoredString {
    s.cyan()
}

pub fn muted(s: &str) -> ColoredString {
    s.dimmed()
}

pub fn strong(s: &str) -> ColoredString {
    s.bold()
}

pub fn success(s: &str) -> ColoredString {
    s.green().bold()
}

pub fn danger(s: &str) -> ColoredString {
    s.red().bold()
}

pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", n, if n == 1 { one } else { many })
}

/// Aligned `label  value` rows. Labels share one column so values form a
/// single left edge.
pub struct Ledger<'a> {
    rows: Vec<(&'a str, String)>,
}

impl<'a> Ledger<'a> {
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }

    pub fn row(mut self, label: &'a str, value: impl Into<String>) -> Self {
        self.rows.push((label, value.into()));
        self
    }
}

impl fmt::Display for Ledger<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let width = self.rows.iter().map(|(l, _)| l.len()).max().unwrap_or(0);
        for (i, (label, value)) in self.rows.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{}  {}", muted(&format!("{label:<width$}")), value)?;
        }
        Ok(())
    }
}

/// An error that carries a short next step for the person running the tool.
#[derive(Debug)]
pub struct UserError {
    pub message: String,
    pub hint: Option<String>,
}

impl fmt::Display for UserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UserError {}

pub fn user_error(message: impl Into<String>, hint: Option<&str>) -> anyhow::Error {
    UserError {
        message: message.into(),
        hint: hint.map(str::to_string),
    }
    .into()
}

pub fn print_error(err: &anyhow::Error) {
    const LABEL: &str = "error";
    let indent = " ".repeat(LABEL.len() + 2);
    eprintln!("{}  {}", danger(LABEL), err);

    if let Some(user) = err.downcast_ref::<UserError>() {
        if let Some(hint) = &user.hint {
            eprintln!("{indent}{}", muted(hint));
        }
        return;
    }

    for cause in err.chain().skip(1) {
        eprintln!("{indent}{}", muted(&cause.to_string()));
    }
}

/// Spinner on stderr. Hidden automatically when stderr is not a terminal and
/// cleared when finished, so it never lingers in scrollback.
pub fn spinner(len: u64, message: &str) -> ProgressBar {
    let pb = ProgressBar::new(len);
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg} {pos}/{len}")
            .expect("valid spinner template")
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "),
    );
    pb.set_message(message.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

/// Spinner for one long step of unknown length, with elapsed time.
pub fn activity(message: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg} {elapsed:.dim}")
            .expect("valid activity template")
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "),
    );
    pb.set_message(message.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

pub fn download_bar(total: Option<u64>) -> ProgressBar {
    match total {
        Some(n) => {
            let pb = ProgressBar::new(n);
            pb.set_style(
                ProgressStyle::with_template("{bar:40.cyan/dim} {bytes:>10} / {total_bytes}")
                    .expect("valid bar template")
                    .progress_chars("━━━"),
            );
            pb
        }
        None => {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::with_template("{spinner:.cyan} {bytes}")
                    .expect("valid spinner template"),
            );
            pb.enable_steady_tick(Duration::from_millis(80));
            pb
        }
    }
}
