//! Hybrid markdown renderer: ANSI escape codes for text, `bat` for code blocks.
//!
//! Instead of rendering the raw markdown source with bat (which shows `**bold**`
//! with colored asterisks), this module renders actual terminal formatting:
//! bold, italic, colored headers, styled lists, inline code backgrounds, etc.
//!
//! Fenced code blocks are extracted and rendered via bat with the correct
//! language for proper syntax highlighting.

use bat::WrappingMode;
use console::style;
use std::io::{self, Write};

use super::output::{env_no_color, Theme};

/// Render markdown content to the terminal with ANSI formatting.
/// Code blocks are handed off to bat for syntax highlighting.
pub fn render_markdown(content: &str, theme: Theme) {
    let segments = split_segments(content);
    for seg in segments {
        match seg {
            Segment::Text(text) => render_text_block(&text),
            Segment::CodeBlock { lang, code } => render_code_block(&lang, &code, theme),
        }
    }
    let _ = io::stdout().flush();
}

enum Segment {
    Text(String),
    CodeBlock { lang: String, code: String },
}

/// Split content into alternating text and code-block segments.
fn split_segments(content: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut text_buf = String::new();
    let mut in_code = false;
    let mut fence_char = ' ';
    let mut fence_len = 0usize;
    let mut code_lang = String::new();
    let mut code_buf = String::new();

    for line in content.lines() {
        let trimmed = line.trim_start();

        if !in_code {
            if let Some((fc, fl, lang)) = detect_opening_fence(trimmed) {
                // Flush accumulated text
                if !text_buf.is_empty() {
                    segments.push(Segment::Text(std::mem::take(&mut text_buf)));
                }
                in_code = true;
                fence_char = fc;
                fence_len = fl;
                code_lang = lang;
                code_buf.clear();
                continue;
            }
            text_buf.push_str(line);
            text_buf.push('\n');
        } else {
            if is_closing_fence(trimmed, fence_char, fence_len) {
                segments.push(Segment::CodeBlock {
                    lang: std::mem::take(&mut code_lang),
                    code: std::mem::take(&mut code_buf),
                });
                in_code = false;
                continue;
            }
            code_buf.push_str(line);
            code_buf.push('\n');
        }
    }

    // Flush remaining
    if in_code {
        // Unclosed code block — render as text with fence
        text_buf.push_str(&format!("```{}\n{}", code_lang, code_buf));
    }
    if !text_buf.is_empty() {
        segments.push(Segment::Text(text_buf));
    }

    segments
}

fn detect_opening_fence(trimmed: &str) -> Option<(char, usize, String)> {
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let fence_len = trimmed.chars().take_while(|&c| c == first).count();
    if fence_len < 3 {
        return None;
    }
    let after = trimmed[fence_len..].trim();
    // Opening fence shouldn't contain the fence char in the info string
    if after.contains(first) {
        return None;
    }
    let lang = after.split_whitespace().next().unwrap_or("").to_string();
    Some((first, fence_len, lang))
}

fn is_closing_fence(trimmed: &str, fence_char: char, fence_len: usize) -> bool {
    let first = match trimmed.chars().next() {
        Some(c) => c,
        None => return false,
    };
    if first != fence_char {
        return false;
    }
    let count = trimmed.chars().take_while(|&c| c == fence_char).count();
    if count < fence_len {
        return false;
    }
    let after = &trimmed[count..];
    after.is_empty() || after.chars().all(|c| c.is_whitespace())
}

// ── Code block rendering (bat) ──────────────────────────────────────────

fn render_code_block(lang: &str, code: &str, theme: Theme) {
    let code = code.trim_end_matches('\n');
    if code.is_empty() {
        return;
    }

    // Header bar
    if lang.is_empty() {
        println!("  {}", style("┌─────").dim());
    } else {
        println!("  {} {}", style("┌─────").dim(), style(lang).cyan().dim());
    }

    let bat_lang = if lang.is_empty() { "txt" } else { lang };
    bat::PrettyPrinter::new()
        .input(bat::Input::from_bytes(code.as_bytes()))
        .theme(theme.as_str())
        .colored_output(env_no_color())
        .language(bat_lang)
        .wrapping_mode(WrappingMode::NoWrapping(true))
        .print()
        .unwrap_or(true);

    println!("  {}", style("└─────").dim());
}

// ── Text rendering (ANSI) ───────────────────────────────────────────────

fn render_text_block(text: &str) {
    for line in text.lines() {
        let rendered = render_line(line);
        println!("{}", rendered);
    }
}

fn render_line(line: &str) -> String {
    if line.is_empty() {
        return String::new();
    }

    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];

    // Horizontal rule
    if is_horizontal_rule(trimmed) {
        return format!(
            "{}  {}",
            indent,
            style("────────────────────────────────────").dim()
        );
    }

    // Headers
    if let Some((level, text)) = parse_header(trimmed) {
        return format!("{}{}", indent, render_header(level, text));
    }

    // Blockquote
    if let Some(rest) = trimmed.strip_prefix("> ") {
        return format!(
            "{}  {} {}",
            indent,
            style("│").cyan().dim(),
            style(render_inline(rest)).italic()
        );
    }
    if trimmed == ">" {
        return format!("{}  {}", indent, style("│").cyan().dim());
    }

    // Unordered list
    if let Some(rest) = strip_ul_marker(trimmed) {
        return format!("{}  {} {}", indent, style("•").cyan(), render_inline(rest));
    }

    // Ordered list
    if let Some((num, rest)) = strip_ol_marker(trimmed) {
        return format!("{}  {}. {}", indent, style(num).cyan(), render_inline(rest));
    }

    // Regular line
    format!("{}{}", indent, render_inline(trimmed))
}

fn is_horizontal_rule(s: &str) -> bool {
    let t = s.trim();
    if t.len() < 3 {
        return false;
    }
    let chars: Vec<char> = t.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.is_empty() {
        return false;
    }
    let first = chars[0];
    (first == '-' || first == '*' || first == '_') && chars.iter().all(|&c| c == first)
}

fn parse_header(s: &str) -> Option<(usize, &str)> {
    if !s.starts_with('#') {
        return None;
    }
    let level = s.chars().take_while(|&c| c == '#').count();
    if level > 6 {
        return None;
    }
    let rest = &s[level..];
    if rest.is_empty() || rest.starts_with(' ') {
        Some((level, rest.trim_start()))
    } else {
        None
    }
}

fn render_header(level: usize, text: &str) -> String {
    let rendered = render_inline(text);
    match level {
        1 => format!(
            "\n  {} {}\n",
            style("█").magenta(),
            style(rendered).bold().magenta()
        ),
        2 => format!(
            "\n  {} {}\n",
            style("▐").magenta().dim(),
            style(rendered).bold().magenta()
        ),
        3 => format!("  {} {}", style("▸").cyan(), style(rendered).bold().cyan()),
        4 => format!("  {}", style(rendered).bold()),
        _ => format!("  {}", style(rendered).bold().dim()),
    }
}

fn strip_ul_marker(s: &str) -> Option<&str> {
    for prefix in &["- ", "* ", "+ "] {
        if s.starts_with(prefix) {
            return Some(s[prefix.len()..].trim_start());
        }
    }
    if (s.starts_with('-') || s.starts_with('*') || s.starts_with('+'))
        && s.len() > 1
        && (s.as_bytes()[1] == b' ' || s.as_bytes()[1] == b'\t')
    {
        return Some(s[1..].trim_start());
    }
    None
}

fn strip_ol_marker(s: &str) -> Option<(u32, &str)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i >= bytes.len() {
        return None;
    }
    if bytes[i] == b'.' && i + 1 < bytes.len() && bytes[i + 1] == b' ' {
        let num: u32 = s[..i].parse().ok()?;
        return Some((num, s[i + 2..].trim_start()));
    }
    None
}

// ── Inline formatting ───────────────────────────────────────────────────

fn render_inline(text: &str) -> String {
    let mut result = String::with_capacity(text.len() + 128);
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Escaped character
        if chars[i] == '\\' && i + 1 < len {
            result.push(chars[i + 1]);
            i += 2;
            continue;
        }

        // Bold+Italic: ***text***
        if i + 2 < len && chars[i] == '*' && chars[i + 1] == '*' && chars[i + 2] == '*' {
            if let Some(end) = find_triple_closing(&chars, i + 3) {
                let inner: String = chars[i + 3..end].iter().collect();
                result.push_str(&format!("{}", style(&inner).bold().italic()));
                i = end + 3;
                continue;
            }
        }

        // Bold: **text**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(end) = find_double_closing(&chars, i + 2, '*') {
                let inner: String = chars[i + 2..end].iter().collect();
                result.push_str(&format!("{}", style(&inner).bold()));
                i = end + 2;
                continue;
            }
        }

        // Bold: __text__
        if i + 1 < len && chars[i] == '_' && chars[i + 1] == '_' {
            if let Some(end) = find_double_closing(&chars, i + 2, '_') {
                let inner: String = chars[i + 2..end].iter().collect();
                result.push_str(&format!("{}", style(&inner).bold()));
                i = end + 2;
                continue;
            }
        }

        // Italic: *text* (not **)
        if chars[i] == '*' && (i + 1 >= len || chars[i + 1] != '*') {
            if let Some(end) = find_single_closing(&chars, i + 1, '*') {
                let inner: String = chars[i + 1..end].iter().collect();
                result.push_str(&format!("{}", style(&inner).italic()));
                i = end + 1;
                continue;
            }
        }

        // Italic: _text_ (not __)
        if chars[i] == '_' && (i + 1 >= len || chars[i + 1] != '_') {
            if let Some(end) = find_single_closing(&chars, i + 1, '_') {
                let inner: String = chars[i + 1..end].iter().collect();
                result.push_str(&format!("{}", style(&inner).italic()));
                i = end + 1;
                continue;
            }
        }

        // Strikethrough: ~~text~~
        if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
            if let Some(end) = find_double_closing(&chars, i + 2, '~') {
                let inner: String = chars[i + 2..end].iter().collect();
                result.push_str(&format!("{}", style(&inner).strikethrough()));
                i = end + 2;
                continue;
            }
        }

        // Inline code: `text`
        if chars[i] == '`' {
            // Count backticks for matching (`` code `` support)
            let tick_count = chars[i..].iter().take_while(|&&c| c == '`').count();
            if let Some(end) = find_backtick_closing(&chars, i + tick_count, tick_count) {
                let inner: String = chars[i + tick_count..end].iter().collect();
                let inner = inner.trim();
                result.push_str(&format!("\x1b[36m\x1b[48;5;236m {} \x1b[0m", inner));
                i = end + tick_count;
                continue;
            }
        }

        // Link: [text](url)
        if chars[i] == '[' {
            if let Some((text_end, url_end)) = find_link(&chars, i) {
                let link_text: String = chars[i + 1..text_end].iter().collect();
                let url: String = chars[text_end + 2..url_end].iter().collect();
                result.push_str(&format!(
                    "{} {}",
                    style(&link_text).underlined().cyan(),
                    style(format!("({})", url)).dim()
                ));
                i = url_end + 1;
                continue;
            }
        }

        // Image: ![alt](url) — show as link
        if chars[i] == '!' && i + 1 < len && chars[i + 1] == '[' {
            if let Some((text_end, url_end)) = find_link(&chars, i + 1) {
                let alt: String = chars[i + 2..text_end].iter().collect();
                let url: String = chars[text_end + 2..url_end].iter().collect();
                result.push_str(&format!(
                    "{} {} {}",
                    style("🖼").dim(),
                    style(&alt).dim(),
                    style(format!("({})", url)).dim()
                ));
                i = url_end + 1;
                continue;
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

fn find_triple_closing(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 2 < chars.len() {
        if chars[i] == '*' && chars[i + 1] == '*' && chars[i + 2] == '*' && i > start {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_double_closing(chars: &[char], start: usize, marker: char) -> Option<usize> {
    let mut i = start;
    while i + 1 < chars.len() {
        if chars[i] == marker && chars[i + 1] == marker && i > start {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_single_closing(chars: &[char], start: usize, marker: char) -> Option<usize> {
    for i in start..chars.len() {
        if chars[i] == marker && (i == start || chars[i - 1] != '\\') && i > start {
            return Some(i);
        }
    }
    None
}

fn find_backtick_closing(chars: &[char], start: usize, tick_count: usize) -> Option<usize> {
    let mut i = start;
    while i + tick_count <= chars.len() {
        let is_match = chars[i..i + tick_count].iter().all(|&c| c == '`');
        let not_more = i + tick_count >= chars.len() || chars[i + tick_count] != '`';
        if is_match && not_more && i > start {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_link(chars: &[char], start: usize) -> Option<(usize, usize)> {
    let mut i = start + 1;
    let mut depth = 1;
    while i < chars.len() && depth > 0 {
        if chars[i] == '[' {
            depth += 1;
        } else if chars[i] == ']' {
            depth -= 1;
        }
        if depth == 0 {
            break;
        }
        i += 1;
    }
    if i >= chars.len() || depth != 0 {
        return None;
    }
    let text_end = i;
    if text_end + 1 >= chars.len() || chars[text_end + 1] != '(' {
        return None;
    }
    i = text_end + 2;
    let mut paren_depth = 1;
    while i < chars.len() && paren_depth > 0 {
        if chars[i] == '(' {
            paren_depth += 1;
        } else if chars[i] == ')' {
            paren_depth -= 1;
        }
        if paren_depth == 0 {
            break;
        }
        i += 1;
    }
    if paren_depth != 0 {
        return None;
    }
    Some((text_end, i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_segments_no_code() {
        let segments = split_segments("Hello world\nFoo bar\n");
        assert_eq!(segments.len(), 1);
        assert!(matches!(&segments[0], Segment::Text(t) if t.contains("Hello")));
    }

    #[test]
    fn test_split_segments_with_code() {
        let input = "Before\n```rust\nfn main() {}\n```\nAfter\n";
        let segments = split_segments(input);
        assert_eq!(segments.len(), 3);
        assert!(matches!(&segments[0], Segment::Text(t) if t.contains("Before")));
        assert!(
            matches!(&segments[1], Segment::CodeBlock { lang, code } if lang == "rust" && code.contains("fn main"))
        );
        assert!(matches!(&segments[2], Segment::Text(t) if t.contains("After")));
    }

    #[test]
    fn test_render_inline_bold() {
        let result = render_inline("Hello **world**!");
        assert!(result.contains("world"));
        // Bold text should not contain the raw ** markers
        assert!(!result.contains("**"));
    }

    #[test]
    fn test_render_inline_code() {
        let result = render_inline("Use `println!` macro");
        assert!(result.contains("println!"));
        // Inline code uses raw ANSI escapes (not console::style) so always present
        assert!(result.contains("\x1b[36m"));
    }

    #[test]
    fn test_parse_header() {
        assert_eq!(parse_header("# Title"), Some((1, "Title")));
        assert_eq!(parse_header("## Sub"), Some((2, "Sub")));
        assert_eq!(parse_header("### Deep"), Some((3, "Deep")));
        assert_eq!(parse_header("Not header"), None);
        assert_eq!(parse_header("#nospace"), None);
    }

    #[test]
    fn test_horizontal_rule() {
        assert!(is_horizontal_rule("---"));
        assert!(is_horizontal_rule("***"));
        assert!(is_horizontal_rule("___"));
        assert!(is_horizontal_rule("- - -"));
        assert!(!is_horizontal_rule("--"));
        assert!(!is_horizontal_rule("hello"));
    }

    #[test]
    fn test_strip_ul_marker() {
        assert_eq!(strip_ul_marker("- item"), Some("item"));
        assert_eq!(strip_ul_marker("* item"), Some("item"));
        assert_eq!(strip_ul_marker("+ item"), Some("item"));
        assert_eq!(strip_ul_marker("no bullet"), None);
    }

    #[test]
    fn test_strip_ol_marker() {
        assert_eq!(strip_ol_marker("1. First"), Some((1, "First")));
        assert_eq!(strip_ol_marker("42. Deep"), Some((42, "Deep")));
        assert_eq!(strip_ol_marker("not a list"), None);
    }
}
