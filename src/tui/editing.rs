use crossterm::event::{KeyCode, KeyModifiers};

/// Emacs-style text editing on a String buffer with cursor position.
/// Returns true if the key was consumed.
pub(super) fn emacs_edit(
    code: KeyCode,
    modifiers: KeyModifiers,
    buf: &mut String,
    pos: &mut usize,
    accept_char: bool,
) -> bool {
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    // Clamp cursor to a UTF-8 character boundary.
    *pos = nearest_prev_char_boundary(buf, *pos);
    if *pos > buf.len() {
        *pos = buf.len();
    }
    match code {
        KeyCode::Char('a') if ctrl => {
            *pos = 0;
            true
        }
        KeyCode::Char('e') if ctrl => {
            *pos = buf.len();
            true
        }
        KeyCode::Char('b') if ctrl => {
            *pos = prev_char_boundary(buf, *pos);
            true
        }
        KeyCode::Char('f') if ctrl => {
            *pos = next_char_boundary(buf, *pos);
            true
        }
        KeyCode::Char('d') if ctrl => {
            if *pos < buf.len() {
                let next = next_char_boundary(buf, *pos);
                if next > *pos {
                    buf.drain(*pos..next);
                }
            }
            true
        }
        KeyCode::Char('k') if ctrl => {
            buf.truncate(*pos);
            true
        }
        KeyCode::Char('u') if ctrl => {
            if *pos > 0 {
                buf.drain(..*pos);
                *pos = 0;
            }
            true
        }
        KeyCode::Char('w') if ctrl => {
            let mut new_pos = None;
            for (idx, ch) in buf[..*pos].char_indices().rev() {
                if !(ch.is_alphanumeric() || ch == '_' || ch == '-') {
                    new_pos = Some(idx + ch.len_utf8());
                    break;
                }
            }
            if let Some(new_pos) = new_pos {
                if new_pos < *pos {
                    buf.drain(new_pos..*pos);
                    *pos = new_pos;
                }
            } else if *pos > 0 {
                buf.drain(..*pos);
                *pos = 0;
            }
            true
        }
        KeyCode::Backspace => {
            let prev = prev_char_boundary(buf, *pos);
            if prev < *pos {
                buf.drain(prev..*pos);
                *pos = prev;
            }
            true
        }
        KeyCode::Char('h') if ctrl => {
            let prev = prev_char_boundary(buf, *pos);
            if prev < *pos {
                buf.drain(prev..*pos);
                *pos = prev;
            }
            true
        }
        KeyCode::Char('\u{8}') | KeyCode::Char('\u{7f}') => {
            let prev = prev_char_boundary(buf, *pos);
            if prev < *pos {
                buf.drain(prev..*pos);
                *pos = prev;
            }
            true
        }
        KeyCode::Left => {
            *pos = prev_char_boundary(buf, *pos);
            true
        }
        KeyCode::Right => {
            *pos = next_char_boundary(buf, *pos);
            true
        }
        KeyCode::Home => {
            *pos = 0;
            true
        }
        KeyCode::End => {
            *pos = buf.len();
            true
        }
        KeyCode::Delete => {
            if *pos < buf.len() {
                let next = next_char_boundary(buf, *pos);
                if next > *pos {
                    buf.drain(*pos..next);
                }
            }
            true
        }
        KeyCode::Char(c) if accept_char => {
            buf.insert(*pos, c);
            *pos = next_char_boundary(buf, *pos);
            true
        }
        _ => false,
    }
}

fn prev_char_boundary(s: &str, pos: usize) -> usize {
    let mut pos = nearest_prev_char_boundary(s, pos);
    if pos == 0 {
        return 0;
    }
    pos -= 1;
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

fn nearest_prev_char_boundary(s: &str, pos: usize) -> usize {
    let mut pos = pos.min(s.len());
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

fn next_char_boundary(s: &str, pos: usize) -> usize {
    let pos = pos.min(s.len());
    if pos >= s.len() {
        return s.len();
    }
    let mut next = pos + 1;
    while next < s.len() && !s.is_char_boundary(next) {
        next += 1;
    }
    next
}

pub(super) fn display_with_cursor(value: &str, cursor_pos: usize) -> String {
    let pos = nearest_prev_char_boundary(value, cursor_pos);
    format!("{}█{}", &value[..pos], &value[pos..])
}

pub(super) fn insert_str_at_cursor(buf: &mut String, cursor_pos: &mut usize, text: &str) {
    *cursor_pos = nearest_prev_char_boundary(buf, *cursor_pos);
    buf.insert_str(*cursor_pos, text);
    *cursor_pos += text.len();
}

pub(super) fn insert_filtered_str_at_cursor(
    buf: &mut String,
    cursor_pos: &mut usize,
    text: &str,
    keep: impl Fn(char) -> bool,
) {
    let filtered: String = text.chars().filter(|ch| keep(*ch)).collect();
    insert_str_at_cursor(buf, cursor_pos, &filtered);
}

pub(super) fn is_alias_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'
}

pub(super) fn provider_key_cursor_pos(step: usize, key_name: &str, key: &str) -> usize {
    match step {
        0 => key_name.len(),
        _ => key.len(),
    }
}

pub(super) fn provider_add_cursor_pos(
    step: usize,
    existing_id: Option<&str>,
    name: &str,
    url: &str,
    key_name: &str,
    key: &str,
) -> usize {
    match step {
        0 if existing_id.is_some() => key_name.len(),
        0 => name.len(),
        1 => url.len(),
        2 => key_name.len(),
        _ => key.len(),
    }
}

pub(super) fn visible_window(selected: usize, total: usize, page_size: usize) -> (usize, usize) {
    if total == 0 {
        return (0, 0);
    }
    let page_size = page_size.max(1);
    let start = (selected / page_size) * page_size;
    let end = (start + page_size).min(total);
    (start, end)
}

pub(super) fn provider_edit_cursor_pos(step: usize, name: &str, url: &str) -> usize {
    match step {
        0 => name.len(),
        1 => url.len(),
        _ => 0,
    }
}
