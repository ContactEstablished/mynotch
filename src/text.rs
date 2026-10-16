//! A single-line text field: the editing model, key mapping, a UTF-16 joiner for `WM_CHAR`,
//! and drawing. Indices are byte offsets on char boundaries; grapheme clusters are out of
//! scope.

use std::ops::Range;

use crate::gfx::{Align, Canvas, Font, Ink, Rect, Rgb};

/// Room so the caret isn't clipped at the field's right edge.
const CARET_ROOM: f32 = 2.0;
/// Behind selected text.
const SELECTION: Rgb = crate::theme::color::SELECTION;
/// Space above and below the caret and the selection, inside the field.
const INSET: f32 = 7.0;

// Virtual-key codes, as plain numbers so the mapping is testable without Win32 types.
const VK_BACK: u16 = 0x08;
const VK_RETURN: u16 = 0x0D;
const VK_ESCAPE: u16 = 0x1B;
const VK_END: u16 = 0x23;
const VK_HOME: u16 = 0x24;
const VK_LEFT: u16 = 0x25;
const VK_RIGHT: u16 = 0x27;
const VK_INSERT: u16 = 0x2D;
const VK_DELETE: u16 = 0x2E;
const VK_A: u16 = 0x41;
const VK_C: u16 = 0x43;
const VK_V: u16 = 0x56;
const VK_X: u16 = 0x58;

/// One line of editable text with a caret and an optional selection.
pub struct TextField {
    text: String,
    /// Where typing goes; the moving end of the selection.
    caret: usize,
    /// The fixed end of the selection; equal to `caret` when nothing is selected.
    anchor: usize,
    max_chars: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    Home,
    End,
}

/// What an editing key asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Move the caret; `true` extends the selection (Shift held).
    Move(Motion, bool),
    /// `true` = by word (Ctrl held).
    Backspace(bool),
    Delete(bool),
    SelectAll,
    Copy,
    Cut,
    Paste,
    Submit,
    Cancel,
}

impl TextField {
    pub fn new(max_chars: usize) -> Self {
        Self { text: String::new(), caret: 0, anchor: 0, max_chars }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    pub fn selected(&self) -> &str {
        &self.text[self.selection()]
    }

    /// Types `s` over the selection. Line breaks and tabs become spaces, other control
    /// characters are dropped, and the text is cut short at `max_chars`.
    pub fn insert(&mut self, s: &str) {
        let clean = sanitise(s);
        let sel = self.selection();
        let budget = (self.max_chars + self.text[sel.clone()].chars().count()).saturating_sub(self.text.chars().count());
        let kept: String = clean.chars().take(budget).collect();
        if kept.is_empty() {
            return;
        }
        self.text.replace_range(sel.clone(), &kept);
        self.caret = sel.start + kept.len();
        self.anchor = self.caret;
    }

    /// Deletes the selection, else the char (or word) before the caret.
    pub fn backspace(&mut self, word: bool) {
        if self.delete_selection() || self.caret == 0 {
            return;
        }
        let start = if word { word_left(&self.text, self.caret) } else { prev_boundary(&self.text, self.caret) };
        self.remove(start..self.caret);
    }

    /// Deletes the selection, else the char (or word) after the caret.
    pub fn delete(&mut self, word: bool) {
        if self.delete_selection() || self.caret == self.text.len() {
            return;
        }
        let end = if word { word_right(&self.text, self.caret) } else { next_boundary(&self.text, self.caret) };
        self.remove(self.caret..end);
    }

    pub fn move_caret(&mut self, m: Motion, extend: bool) {
        if !extend && self.anchor != self.caret && matches!(m, Motion::Left | Motion::Right) {
            // Left/Right without Shift first collapse the selection to that side.
            let sel = self.selection();
            self.caret = if m == Motion::Left { sel.start } else { sel.end };
            self.anchor = self.caret;
            return;
        }
        self.caret = match m {
            Motion::Left => prev_boundary(&self.text, self.caret),
            Motion::Right => next_boundary(&self.text, self.caret),
            Motion::WordLeft => word_left(&self.text, self.caret),
            Motion::WordRight => word_right(&self.text, self.caret),
            Motion::Home => 0,
            Motion::End => self.text.len(),
        };
        if !extend {
            self.anchor = self.caret;
        }
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// Removes and returns the selection; `None` if nothing is selected.
    pub fn cut(&mut self) -> Option<String> {
        let sel = self.selection();
        if sel.is_empty() {
            return None;
        }
        let cut = self.text[sel.clone()].to_string();
        self.remove(sel);
        Some(cut)
    }

    /// Puts the caret at byte `at` (clamped, and rounded down to a char boundary), extending
    /// the selection if asked.
    pub fn set_caret(&mut self, at: usize, extend: bool) {
        let mut at = at.min(self.text.len());
        while !self.text.is_char_boundary(at) {
            at -= 1;
        }
        self.caret = at;
        if !extend {
            self.anchor = at;
        }
    }

    fn delete_selection(&mut self) -> bool {
        let sel = self.selection();
        if sel.is_empty() {
            return false;
        }
        self.remove(sel);
        true
    }

    fn remove(&mut self, range: Range<usize>) {
        self.caret = range.start;
        self.anchor = range.start;
        self.text.replace_range(range, "");
    }
}

fn sanitise(s: &str) -> String {
    s.replace("\r\n", " ")
        .chars()
        .filter_map(|c| match c {
            '\r' | '\n' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

fn prev_boundary(text: &str, from: usize) -> usize {
    text[..from].chars().next_back().map_or(from, |c| from - c.len_utf8())
}

fn next_boundary(text: &str, from: usize) -> usize {
    text[from..].chars().next().map_or(from, |c| from + c.len_utf8())
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The start of the word before `from`, skipping any spaces and punctuation first.
fn word_left(text: &str, from: usize) -> usize {
    let mut at = from;
    let mut chars = text[..from].char_indices().rev().peekable();
    while let Some((i, _)) = chars.next_if(|&(_, c)| !is_word(c)) {
        at = i;
    }
    while let Some((i, _)) = chars.next_if(|&(_, c)| is_word(c)) {
        at = i;
    }
    at
}

/// The start of the next word after `from` (past the rest of this word and any spaces and
/// punctuation), as Notepad's Ctrl+Right goes.
fn word_right(text: &str, from: usize) -> usize {
    let rest = &text[from..];
    let mut chars = rest.char_indices().peekable();
    while chars.next_if(|&(_, c)| is_word(c)).is_some() {}
    while chars.next_if(|&(_, c)| !is_word(c)).is_some() {}
    from + chars.peek().map_or(rest.len(), |&(i, _)| i)
}

/// The editing command for a key press, if it is one. `ctrl` must be false for Ctrl+Alt
/// chords: AltGr arrives as Ctrl+Alt and types characters on many layouts.
pub fn command(vk: u16, ctrl: bool, shift: bool) -> Option<Command> {
    Some(match vk {
        VK_LEFT if ctrl => Command::Move(Motion::WordLeft, shift),
        VK_RIGHT if ctrl => Command::Move(Motion::WordRight, shift),
        VK_LEFT => Command::Move(Motion::Left, shift),
        VK_RIGHT => Command::Move(Motion::Right, shift),
        VK_HOME => Command::Move(Motion::Home, shift),
        VK_END => Command::Move(Motion::End, shift),
        VK_BACK => Command::Backspace(ctrl),
        VK_DELETE if shift && !ctrl => Command::Cut,
        VK_DELETE => Command::Delete(ctrl),
        VK_INSERT if ctrl => Command::Copy,
        VK_INSERT if shift => Command::Paste,
        VK_A if ctrl => Command::SelectAll,
        VK_C if ctrl => Command::Copy,
        VK_X if ctrl => Command::Cut,
        VK_V if ctrl => Command::Paste,
        VK_RETURN => Command::Submit,
        VK_ESCAPE => Command::Cancel,
        _ => return None,
    })
}

/// Turns `WM_CHAR` UTF-16 code units into chars, pairing surrogates.
#[derive(Default)]
pub struct Utf16Joiner {
    high: Option<u16>,
}

impl Utf16Joiner {
    /// The char completed by `unit`, if any. Control characters (Backspace, Enter, Esc, Tab,
    /// Ctrl+letter codes) give `None`: the key-down handler deals with those keys.
    pub fn push(&mut self, unit: u16) -> Option<char> {
        match unit {
            0xD800..=0xDBFF => {
                self.high = Some(unit);
                None
            }
            0xDC00..=0xDFFF => {
                let high = self.high.take()?;
                char::decode_utf16([high, unit]).next()?.ok()
            }
            _ => {
                self.high = None;
                char::from_u32(u32::from(unit)).filter(|c| !c.is_control())
            }
        }
    }
}

/// Draws `field` in `r`: the text (or `placeholder` when empty), the selection, and the caret
/// when `caret_on`. Long text scrolls so the caret stays visible.
pub fn draw(cv: &Canvas, field: &TextField, r: Rect, font: Font, placeholder: &str, caret_on: bool) {
    let caret_x = cv.measure(&field.text[..field.caret], font);
    let off = scroll_offset(caret_x, (r.w - CARET_ROOM).max(0.0));
    cv.push_clip(r);
    let sel = field.selection();
    if !sel.is_empty() {
        let x0 = cv.measure(&field.text[..sel.start], font);
        let x1 = cv.measure(&field.text[..sel.end], font);
        let band = Rect::new(r.x + x0 - off, r.y + INSET, x1 - x0, r.h - INSET * 2.0);
        cv.fill(band, 2.0, Ink::Rgb(SELECTION));
    }
    if field.text().is_empty() {
        cv.text(placeholder, font, Ink::Dim, r, Align::Left);
    } else {
        // At least as wide as the text, so `Canvas::text` doesn't trim it with an ellipsis;
        // the clip does the cutting.
        let full = cv.measure(&field.text, font);
        cv.text(&field.text, font, Ink::Text, Rect::new(r.x - off, r.y, full + CARET_ROOM * 2.0, r.h), Align::Left);
    }
    if caret_on {
        let x = r.x + caret_x - off;
        cv.line((x, r.y + INSET), (x, r.y + r.h - INSET), Ink::Text);
    }
    cv.pop_clip();
}

/// The caret position nearest `x`, for a click in a field drawn by `draw` in `r`.
pub fn caret_at(cv: &Canvas, field: &TextField, r: Rect, font: Font, x: f32) -> usize {
    let off = scroll_offset(cv.measure(&field.text[..field.caret], font), (r.w - CARET_ROOM).max(0.0));
    nearest_boundary(&field.text, x - r.x + off, |s| cv.measure(s, font))
}

/// How far the text is shifted left so the caret stays visible: 0 until the caret passes the
/// right edge, then exactly enough to keep it there.
fn scroll_offset(caret_x: f32, visible_w: f32) -> f32 {
    (caret_x - visible_w).max(0.0)
}

/// The char boundary whose prefix is closest to `x` wide (the first, on a tie).
fn nearest_boundary(text: &str, x: f32, measure: impl Fn(&str) -> f32) -> usize {
    let boundaries = text.char_indices().map(|(i, _)| i).chain([text.len()]);
    let mut best = (0, f32::INFINITY);
    for i in boundaries {
        let d = (measure(&text[..i]) - x).abs();
        if d < best.1 {
            best = (i, d);
        }
    }
    best.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(text: &str) -> TextField {
        let mut f = TextField::new(100);
        f.insert(text);
        f
    }

    fn on_boundaries(f: &TextField) -> bool {
        f.text.is_char_boundary(f.caret) && f.text.is_char_boundary(f.anchor)
    }

    #[test]
    fn insert_replaces_the_selection_and_sanitises_control_chars() {
        let f = field("a\r\nb\tc\u{7}d");
        assert_eq!(f.text(), "a b cd");
        let mut f = field("hello world");
        f.move_caret(Motion::Home, false);
        f.move_caret(Motion::WordRight, true);
        assert_eq!(f.selected(), "hello ");
        f.insert("bye ");
        assert_eq!(f.text(), "bye world");
        assert_eq!((f.caret, f.anchor), (4, 4), "caret after the insert");
    }

    #[test]
    fn insert_respects_max_chars() {
        let mut f = TextField::new(5);
        f.insert("abcdefg");
        assert_eq!(f.text(), "abcde");
        f.set_caret(1, false);
        f.set_caret(4, true);
        f.insert("0123456789");
        assert_eq!(f.text(), "a012e", "3 selected chars make room for 3");
        f.move_caret(Motion::End, false);
        f.insert("x");
        assert_eq!(f.text(), "a012e", "full: nothing more fits");
        f.insert("\u{7}");
        assert_eq!(f.text(), "a012e");
    }

    #[test]
    fn backspace_and_delete_by_char_and_by_word() {
        let mut f = field("hello, world");
        f.backspace(false);
        assert_eq!(f.text(), "hello, worl");
        f.backspace(true);
        assert_eq!(f.text(), "hello, ");
        f.backspace(true);
        assert_eq!(f.text(), "");
        f.backspace(false);
        assert_eq!(f.text(), "", "nothing before the caret");

        let mut f = field("hello, world");
        f.move_caret(Motion::Home, false);
        f.delete(false);
        assert_eq!(f.text(), "ello, world");
        f.delete(true);
        assert_eq!(f.text(), "world");
        f.move_caret(Motion::End, false);
        f.delete(true);
        assert_eq!(f.text(), "world", "nothing after the caret");

        let mut f = field("abcdef");
        f.set_caret(1, false);
        f.set_caret(3, true);
        f.backspace(true);
        assert_eq!(f.text(), "adef", "a selection goes whatever `word` says");
        f.set_caret(1, false);
        f.set_caret(3, true);
        f.delete(true);
        assert_eq!(f.text(), "af");
    }

    #[test]
    fn caret_motions_extend_and_collapse_the_selection() {
        let mut f = field("abcdef");
        f.move_caret(Motion::Left, true);
        f.move_caret(Motion::Left, true);
        assert_eq!(f.selected(), "ef");
        f.move_caret(Motion::Left, false);
        assert_eq!((f.caret, f.anchor), (4, 4), "Left collapses to the selection's start");
        f.move_caret(Motion::Right, true);
        f.move_caret(Motion::Right, true);
        f.move_caret(Motion::Right, false);
        assert_eq!((f.caret, f.anchor), (6, 6), "Right collapses to its end");
        f.move_caret(Motion::Home, true);
        assert_eq!(f.selected(), "abcdef");
        f.move_caret(Motion::End, true);
        assert_eq!(f.selected(), "");
        f.move_caret(Motion::Home, false);
        f.move_caret(Motion::Right, false);
        assert_eq!(f.caret, 1);
    }

    #[test]
    fn word_motions_skip_spaces_and_punctuation() {
        let t = "hello, world";
        assert_eq!(word_right(t, 0), 7);
        assert_eq!(word_right(t, 7), 12);
        assert_eq!(word_left(t, 7), 0);
        assert_eq!(word_left(t, 12), 7);
        assert_eq!(word_left(t, 0), 0);
        let t = "  two  words  ";
        assert_eq!(word_right(t, 0), 2);
        assert_eq!(word_right(t, 2), 7);
        assert_eq!(word_right(t, 7), 14);
        assert_eq!(word_left(t, 14), 7);
        assert_eq!(word_left(t, 2), 0);
        assert_eq!(word_right("snake_case", 0), 10, "underscores join words");
    }

    #[test]
    fn indices_stay_on_char_boundaries_with_multibyte_text() {
        let mut f = field("aé😀中b");
        for m in [Motion::Left, Motion::Left, Motion::WordLeft, Motion::Right, Motion::WordRight, Motion::Home] {
            f.move_caret(m, true);
            assert!(on_boundaries(&f), "{m:?}");
        }
        f.move_caret(Motion::End, false);
        f.move_caret(Motion::Left, false);
        f.backspace(false);
        assert_eq!(f.text(), "aé😀b", "backspace removes the whole char");
        f.backspace(false);
        assert_eq!(f.text(), "aéb");
        f.move_caret(Motion::Home, false);
        f.move_caret(Motion::Right, false);
        f.delete(false);
        assert_eq!(f.text(), "ab");
        assert!(on_boundaries(&f));
    }

    #[test]
    fn cut_and_selected_return_the_selection() {
        let mut f = field("abcdef");
        assert_eq!(f.cut(), None);
        f.set_caret(4, false);
        f.set_caret(2, true);
        assert_eq!(f.selected(), "cd");
        assert_eq!(f.cut().as_deref(), Some("cd"));
        assert_eq!(f.text(), "abef");
        assert_eq!((f.caret, f.anchor), (2, 2));
    }

    #[test]
    fn set_caret_clamps_and_rounds_down_to_a_char_boundary() {
        let mut f = field("aéb");
        f.set_caret(99, false);
        assert_eq!(f.caret, 4);
        f.set_caret(2, false);
        assert_eq!((f.caret, f.anchor), (1, 1), "inside é rounds down");
        f.set_caret(4, true);
        assert_eq!((f.anchor, f.caret), (1, 4), "extend keeps the anchor");
    }

    #[test]
    fn command_maps_editing_keys() {
        use Command::*;
        assert_eq!(command(VK_LEFT, false, false), Some(Move(Motion::Left, false)));
        assert_eq!(command(VK_RIGHT, false, true), Some(Move(Motion::Right, true)));
        assert_eq!(command(VK_LEFT, true, true), Some(Move(Motion::WordLeft, true)));
        assert_eq!(command(VK_RIGHT, true, false), Some(Move(Motion::WordRight, false)));
        assert_eq!(command(VK_HOME, true, true), Some(Move(Motion::Home, true)));
        assert_eq!(command(VK_END, false, false), Some(Move(Motion::End, false)));
        assert_eq!(command(VK_BACK, true, false), Some(Backspace(true)));
        assert_eq!(command(VK_DELETE, false, false), Some(Delete(false)));
        assert_eq!(command(VK_DELETE, true, false), Some(Delete(true)));
        assert_eq!(command(VK_DELETE, false, true), Some(Cut));
        assert_eq!(command(VK_INSERT, true, false), Some(Copy));
        assert_eq!(command(VK_INSERT, false, true), Some(Paste));
        assert_eq!(command(VK_INSERT, false, false), None);
        assert_eq!(command(VK_A, true, false), Some(SelectAll));
        assert_eq!(command(VK_C, true, false), Some(Copy));
        assert_eq!(command(VK_X, true, false), Some(Cut));
        assert_eq!(command(VK_V, true, false), Some(Paste));
        assert_eq!(command(VK_A, false, false), None, "plain letters are typed, not commands");
        assert_eq!(command(VK_RETURN, false, false), Some(Submit));
        assert_eq!(command(VK_ESCAPE, false, false), Some(Cancel));
        assert_eq!(command(0x26, false, false), None, "Up is not an editing key");
    }

    #[test]
    fn joiner_pairs_surrogates_and_drops_controls_and_orphans() {
        let mut j = Utf16Joiner::default();
        assert_eq!(j.push(0xD83D), None);
        assert_eq!(j.push(0xDE00), Some('😀'));
        assert_eq!(j.push(0xDE00), None, "a lone low surrogate");
        assert_eq!(j.push(0xD83D), None);
        assert_eq!(j.push(u16::from(b'a')), Some('a'), "an unpaired high surrogate is dropped");
        for control in [0x08, 0x09, 0x0D, 0x1B, 0x7F, 0x01] {
            assert_eq!(j.push(control), None, "{control:#x}");
        }
        assert_eq!(j.push(0xE9), Some('é'));
    }

    #[test]
    fn scroll_offset_keeps_the_caret_visible() {
        assert_eq!(scroll_offset(50.0, 100.0), 0.0);
        assert_eq!(scroll_offset(150.0, 100.0), 50.0);
    }

    #[test]
    fn nearest_boundary_picks_the_closest_char_edge() {
        let measure = |s: &str| s.chars().count() as f32 * 10.0;
        assert_eq!(nearest_boundary("abcd", 14.0, measure), 1);
        assert_eq!(nearest_boundary("abcd", 16.0, measure), 2);
        assert_eq!(nearest_boundary("abcd", 15.0, measure), 1, "a tie goes to the first");
        assert_eq!(nearest_boundary("abcd", -5.0, measure), 0);
        assert_eq!(nearest_boundary("abcd", 100.0, measure), 4);
        assert_eq!(nearest_boundary("aé😀b", 26.0, measure), 7);
        assert_eq!(nearest_boundary("", 26.0, measure), 0);
    }
}
