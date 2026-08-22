//! Pure dashboard view model.
//!
//! The view layer intentionally has no terminal backend dependency. Dynamic
//! strings are normalized before they reach crossterm so peer-controlled data
//! cannot emit terminal control sequences or bidi/invisible formatting controls.

use p2p_net::NodeSnapshot;

mod sections;
mod services;
mod text;
mod widgets;

pub(crate) use text::sanitize_terminal_text;

const MAX_DASHBOARD_WIDTH: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Text,
    Muted,
    Accent,
    Good,
    Warn,
    Bad,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Span {
    text: String,
    tone: Tone,
    bold: bool,
}

impl Span {
    pub(super) fn new(text: impl AsRef<str>, tone: Tone, bold: bool) -> Self {
        Self {
            text: text::sanitize_terminal_text(text.as_ref()),
            tone,
            bold,
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn tone(&self) -> Tone {
        self.tone
    }

    pub(crate) fn bold(&self) -> bool {
        self.bold
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Line {
    pub(super) spans: Vec<Span>,
}

impl Line {
    pub(super) fn push(&mut self, text: impl AsRef<str>, tone: Tone) {
        self.spans.push(Span::new(text, tone, false));
    }

    pub(super) fn push_bold(&mut self, text: impl AsRef<str>, tone: Tone) {
        self.spans.push(Span::new(text, tone, true));
    }

    pub(crate) fn spans(&self) -> &[Span] {
        &self.spans
    }

    pub(crate) fn visible_width(&self) -> usize {
        self.spans
            .iter()
            .map(|span| text::char_width(&span.text))
            .sum()
    }

    #[cfg(test)]
    pub(crate) fn plain_text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }
}

pub(crate) fn dashboard_lines(snap: &NodeSnapshot, columns: usize, rows: usize) -> Vec<Line> {
    let width = columns.min(MAX_DASHBOARD_WIDTH);
    if rows == 0 || width == 0 {
        return Vec::new();
    }

    let compact = rows < 24 || width < 64;
    let mut lines = Vec::with_capacity(rows.min(64));
    sections::append_header(&mut lines, snap, width);

    if compact {
        sections::append_compact_summary(&mut lines, snap, width);
    } else {
        sections::append_reachability(&mut lines, snap, width);
        sections::append_peer_mesh(&mut lines, snap, width);
        services::append_services(&mut lines, snap, width);
    }

    sections::append_events(&mut lines, snap, width, rows);
    lines.truncate(rows);
    lines
}
