use super::Tone;

const MAX_DISPLAY_FIELD_CHARS: usize = 2_048;

pub(super) fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let secs = seconds % 60;
    if days > 0 {
        format!("{days}d {hours:02}h {minutes:02}m")
    } else {
        format!("{hours:02}:{minutes:02}:{secs:02}")
    }
}

pub(super) fn count_tone(count: usize) -> Tone {
    if count == 0 {
        Tone::Muted
    } else {
        Tone::Good
    }
}

pub(super) fn failure_tone(count: usize) -> Tone {
    if count == 0 {
        Tone::Muted
    } else {
        Tone::Warn
    }
}

pub(super) fn status_tone(value: &str) -> Tone {
    let value = sanitize_terminal_text(value).to_ascii_lowercase();
    if value.contains("error") || value.contains("fail") || value.contains("denied") {
        Tone::Bad
    } else if value.contains("at_capacity")
        || value.contains("rate_limited")
        || value.contains("closed")
        || value.contains("unknown")
        || value.contains("pending")
        || value.contains("probing")
        || value.contains("private")
        || value.contains("restricted")
        || value.contains("cgnat")
    {
        Tone::Warn
    } else if value.contains("public")
        || value.contains("healthy")
        || value.contains("ready")
        || value.contains("success")
        || value.contains("enabled")
    {
        Tone::Good
    } else if value.contains("disabled") || value == "off" || value == "none" {
        Tone::Muted
    } else {
        Tone::Text
    }
}

pub(super) fn event_tone(value: &str) -> Tone {
    let value = sanitize_terminal_text(value).to_ascii_lowercase();
    if value.contains("error")
        || value.contains("failed")
        || value.contains("denied")
        || value.contains("rejected")
    {
        Tone::Bad
    } else if value.contains("established")
        || value.contains("connected")
        || value.contains("success")
        || value.contains("announced")
    {
        Tone::Good
    } else if value.contains("pending")
        || value.contains("deferred")
        || value.contains("retry")
        || value.contains("rotated")
    {
        Tone::Warn
    } else {
        Tone::Text
    }
}

pub(super) fn wrap_terminal_text(value: &str, width: usize) -> Vec<String> {
    let safe = sanitize_terminal_text(value);
    if width == 0 || safe.is_empty() {
        return vec![String::new()];
    }

    let chars: Vec<char> = safe.chars().collect();
    chars
        .chunks(width)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect()
}

pub(super) fn clip_text(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let safe = sanitize_terminal_text(value);
    if char_width(&safe) <= width {
        return safe;
    }
    if width == 1 {
        return "…".to_string();
    }
    let mut clipped: String = safe.chars().take(width - 1).collect();
    clipped.push('…');
    clipped
}

pub(crate) fn sanitize_terminal_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len().min(MAX_DISPLAY_FIELD_CHARS));
    let mut truncated = false;
    for (index, ch) in value.chars().enumerate() {
        if index >= MAX_DISPLAY_FIELD_CHARS {
            truncated = true;
            break;
        }
        if is_safe_terminal_char(ch) {
            out.push(ch);
        } else {
            out.push('?');
        }
    }
    if truncated {
        out.push('…');
    }
    out
}

fn is_safe_terminal_char(ch: char) -> bool {
    ch.is_ascii_graphic()
        || ch == ' '
        || matches!(ch, '╭' | '╮' | '╰' | '╯' | '─' | '│' | '•' | '…')
}

pub(super) fn char_width(value: &str) -> usize {
    value.chars().count()
}
