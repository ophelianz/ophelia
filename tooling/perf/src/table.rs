use anstyle::{AnsiColor, Style};
use std::{
    fmt,
    io::{self, IsTerminal},
};

use crate::model::Importance;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Plain,
    Label,
    Good,
    Bad,
    Warn,
    Muted,
}

pub(crate) struct Theme {
    enabled: bool,
}

impl Theme {
    pub(crate) fn stdout() -> Self {
        Self {
            enabled: io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }

    fn style(&self, tone: Tone) -> Style {
        match tone {
            Tone::Plain => Style::new(),
            Tone::Label => AnsiColor::Cyan.on_default().bold(),
            Tone::Good => AnsiColor::Green.on_default().bold(),
            Tone::Bad => AnsiColor::Red.on_default().bold(),
            Tone::Warn => AnsiColor::Yellow.on_default().bold(),
            Tone::Muted => AnsiColor::BrightBlack.on_default(),
        }
    }

    fn paint(&self, tone: Tone, text: impl fmt::Display) -> String {
        if self.enabled && tone != Tone::Plain {
            let style = self.style(tone);
            format!("{style}{text}{style:#}")
        } else {
            text.to_string()
        }
    }
}

pub(crate) struct Cell {
    text: String,
    tone: Tone,
}

impl Cell {
    pub(crate) fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: Tone::Plain,
        }
    }

    pub(crate) fn toned(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            tone,
        }
    }
}

pub(crate) fn print(theme: &Theme, headers: &[&str], rows: &[Vec<Cell>]) {
    let mut widths = headers
        .iter()
        .map(|header| header.len())
        .collect::<Vec<_>>();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.text.len());
        }
    }

    for (index, header) in headers.iter().enumerate() {
        if index > 0 {
            print!("  ");
        }
        print!(
            "{}",
            theme.paint(
                Tone::Label,
                format!("{header:<width$}", width = widths[index])
            )
        );
    }
    println!();

    for width in &widths {
        print!("{}  ", theme.paint(Tone::Muted, "-".repeat(*width)));
    }
    println!();

    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if index > 0 {
                print!("  ");
            }
            print!(
                "{}",
                theme.paint(
                    cell.tone,
                    format!("{:<width$}", cell.text, width = widths[index])
                )
            );
        }
        println!();
    }
}

pub(crate) fn format_duration(nanos: u128) -> String {
    if nanos >= 1_000_000_000 {
        format!("{:.3} s", nanos as f64 / 1_000_000_000.0)
    } else if nanos >= 1_000_000 {
        format!("{:.3} ms", nanos as f64 / 1_000_000.0)
    } else if nanos >= 1_000 {
        format!("{:.3} us", nanos as f64 / 1_000.0)
    } else {
        format!("{nanos} ns")
    }
}

pub(crate) fn importance_tone(importance: Importance) -> Tone {
    match importance {
        Importance::Critical => Tone::Bad,
        Importance::Important => Tone::Warn,
        Importance::Average => Tone::Plain,
        Importance::Iffy | Importance::Fluff => Tone::Muted,
    }
}

pub(crate) fn delta_tone(delta: f64) -> Tone {
    if delta < -1.0 {
        Tone::Good
    } else if delta > 1.0 {
        Tone::Bad
    } else {
        Tone::Muted
    }
}
