//! Focused editor for one Commentary draft. Persistence remains the controller's responsibility.

use std::cell::Cell;
use std::cell::RefCell;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, StatefulWidgetRef, Widget, Wrap};
use unicode_segmentation::UnicodeSegmentation;

use crate::bottom_pane::{BottomPaneView, CancellationEvent, TextArea, TextAreaState, ViewCompletion, ViewPresentation};
use crate::keymap::KeymapContextSet;
use crate::keymap::RuntimeKeymap;
use crate::render::renderable::Renderable;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EditorEvent { Changed(String), Save(String), Keep(String), Cancel }

pub(crate) struct CommentaryEditor {
    source: String,
    quotation: String,
    editing: bool,
    editor: TextArea,
    editor_state: RefCell<TextAreaState>,
    quote_offset: Cell<u16>,
    quote_rows: Cell<u16>,
    quote_total: Cell<u16>,
    quote_area: Cell<Rect>,
    completion: Option<ViewCompletion>,
    notify: Box<dyn FnMut(EditorEvent)>,
}

impl CommentaryEditor {
    pub(crate) fn new(source: String, quotation: String, note: &str, editing: bool, notify: Box<dyn FnMut(EditorEvent)>) -> Self {
        let mut editor = TextArea::new();
        editor.set_text_clearing_elements(note);
        editor.set_cursor(note.len());
        Self { source, quotation, editing, editor, editor_state: RefCell::new(TextAreaState::default()), quote_offset: Cell::new(0), quote_rows: Cell::new(0), quote_total: Cell::new(0), quote_area: Cell::new(Rect::default()), completion: None, notify }
    }

    pub(crate) fn set_keymap_bindings(&mut self, keymap: &RuntimeKeymap) { self.editor.set_keymap_bindings(keymap); }

    fn scroll_quote(&self, delta: i32) {
        let max = self.quote_total.get().saturating_sub(self.quote_rows.get());
        self.quote_offset.set((i32::from(self.quote_offset.get()) + delta).clamp(0, i32::from(max)) as u16);
    }

    /// The owner routes mouse events here while this centered dialog has focus.
    pub(crate) fn handle_mouse(&mut self, event: MouseEvent) -> bool {
        let area = self.quote_area.get();
        if area.contains((event.column, event.row).into()) {
            match event.kind {
                MouseEventKind::ScrollUp => { self.scroll_quote(-1); return true; }
                MouseEventKind::ScrollDown => { self.scroll_quote(1); return true; }
                _ => {}
            }
        }
        if self.editor.scroll_mouse(event, &mut self.editor_state.borrow_mut()) { return true; }
        self.editor.handle_mouse(event, *self.editor_state.borrow())
    }

    fn areas(&self, area: Rect) -> (Rect, Rect, Rect) {
        let inner = Rect { x: area.x.saturating_add(1), y: area.y.saturating_add(1), width: area.width.saturating_sub(2), height: area.height.saturating_sub(2) };
        let available = inner.height.saturating_sub(4);
        let total = Renderable::desired_height(&Paragraph::new(self.quotation.as_str()).wrap(Wrap { trim: false }), inner.width.max(1));
        let quote_rows = total.min(6).min(available.saturating_sub(1));
        self.quote_total.set(total);
        self.quote_rows.set(quote_rows);
        self.quote_offset.set(self.quote_offset.get().min(total.saturating_sub(quote_rows)));
        let source = Rect { height: inner.height.min(1), ..inner };
        let quote = Rect { y: source.y.saturating_add(source.height).saturating_add(1), height: quote_rows, ..inner }.intersection(inner);
        let input = Rect { y: quote.y.saturating_add(quote.height).saturating_add(u16::from(available > quote_rows)), height: available.saturating_sub(quote_rows), ..inner };
        (source, quote, input.intersection(inner))
    }
}

impl BottomPaneView for CommentaryEditor {
    fn presentation(&self) -> ViewPresentation { ViewPresentation::Centered }
    fn centered_area(&self, viewport: Rect) -> Option<Rect> {
        let width = ((u32::from(viewport.width) * 80 / 100) as u16).min(viewport.width.saturating_sub(2));
        let height = viewport.height.saturating_sub(2);
        Some(Rect::new(viewport.x + (viewport.width - width) / 2, viewport.y + (viewport.height - height) / 2, width, height))
    }
    fn handle_mouse_event(&mut self, event: MouseEvent) -> bool {
        self.handle_mouse(event);
        true
    }
    fn keymap_contexts(&self) -> KeymapContextSet { KeymapContextSet::new(self.editor.keymap_context()) }
    fn accepts_input_when_disconnected(&self) -> bool { true }
    fn handle_key_event(&mut self, key: KeyEvent) {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) || self.completion.is_some() { return; }
        match (key.code, key.modifiers) {
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if self.editor.text().trim().graphemes(true).count() >= 3 {
                    (self.notify)(EditorEvent::Save(self.editor.text().to_owned()));
                    self.completion = Some(ViewCompletion::Accepted);
                }
            }
            (KeyCode::Enter, KeyModifiers::SHIFT) => { self.editor.insert_str("\n"); (self.notify)(EditorEvent::Changed(self.editor.text().to_owned())); }
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => { (self.notify)(EditorEvent::Keep(self.editor.text().to_owned())); self.completion = Some(ViewCompletion::Accepted); }
            (KeyCode::PageUp, KeyModifiers::ALT) => self.scroll_quote(-i32::from(self.quote_rows.get().max(1))),
            (KeyCode::PageDown, KeyModifiers::ALT) => self.scroll_quote(i32::from(self.quote_rows.get().max(1))),
            _ => {
                let before = self.editor.text().to_owned();
                self.editor.input(key);
                if self.editor.text() != before { (self.notify)(EditorEvent::Changed(self.editor.text().to_owned())); }
            }
        }
    }
    fn handle_paste(&mut self, text: String) -> bool {
        if text.is_empty() || self.completion.is_some() { return false; }
        self.editor.insert_str(&text);
        (self.notify)(EditorEvent::Changed(self.editor.text().to_owned()));
        true
    }
    fn on_ctrl_c(&mut self) -> CancellationEvent { (self.notify)(EditorEvent::Cancel); self.completion = Some(ViewCompletion::Cancelled); CancellationEvent::Handled }
    fn is_complete(&self) -> bool { self.completion.is_some() }
    fn completion(&self) -> Option<ViewCompletion> { self.completion }
}

impl Renderable for CommentaryEditor {
    fn desired_height(&self, width: u16) -> u16 {
        let quote = Renderable::desired_height(&Paragraph::new(self.quotation.as_str()).wrap(Wrap { trim: false }), width.saturating_sub(2).max(1)).min(6);
        6u16.saturating_add(quote).saturating_add(self.editor.desired_height(width.saturating_sub(2).max(1)).clamp(1, 8)).max(10)
    }
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        if area.is_empty() { return; }
        Widget::render(Clear, area, buffer);
        Widget::render(Block::default().borders(Borders::ALL).title(if self.editing { " Edit comment " } else { " Add comment " }), area, buffer);
        if area.width < 8 || area.height < 10 {
            self.quote_area.set(Rect::default());
            let inner = Rect { x: area.x.saturating_add(1), y: area.y.saturating_add(1), width: area.width.saturating_sub(2), height: area.height.saturating_sub(2) };
            Widget::render(Paragraph::new("Enlarge the terminal to edit.\nEsc cancel · Ctrl+D keep draft").wrap(Wrap { trim: false }), inner, buffer);
            return;
        }
        let (source, quote, input) = self.areas(area);
        self.quote_area.set(quote);
        Widget::render(Paragraph::new(self.source.as_str().bold()), source, buffer);
        if quote.height > 0 {
            let title = Rect { y: quote.y.saturating_sub(1), height: 1, ..quote };
            let position = format!("Selected text · lines {}–{} of {} · wheel / Alt+PgUp/PgDn", self.quote_offset.get() + 1, self.quote_offset.get() + quote.height, self.quote_total.get());
            Widget::render(Paragraph::new(position.dim()), title, buffer);
        }
        Widget::render(Paragraph::new(self.quotation.as_str().dim()).wrap(Wrap { trim: false }).scroll((self.quote_offset.get(), 0)), quote, buffer);
        if input.y > source.y && input.y < area.bottom() {
            let label = Rect { y: input.y.saturating_sub(1), height: 1, ..input };
            Widget::render(Paragraph::new("Your observation (minimum 3 characters):".dim()), label, buffer);
        }
        if !input.is_empty() { StatefulWidgetRef::render_ref(&(&self.editor), input, buffer, &mut self.editor_state.borrow_mut()); }
        if area.height > 2 {
            let footer = Rect { x: area.x.saturating_add(1), y: area.bottom().saturating_sub(2), width: area.width.saturating_sub(2), height: 1 };
            Widget::render(Paragraph::new("Enter save · Shift+Enter newline · Ctrl+D draft · Esc cancel"), footer, buffer);
        }
    }
    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        if area.width < 8 || area.height < 10 { return None; }
        let (_, _, input) = self.areas(area);
        if input.is_empty() { None } else { self.editor.cursor_pos_with_state(input, *self.editor_state.borrow()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn editor(note: &str) -> (CommentaryEditor, Arc<Mutex<Vec<EditorEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        (CommentaryEditor::new("source".into(), "quoted passage".into(), note, false, Box::new(move |event| sink.lock().unwrap().push(event))), events)
    }

    #[test]
    fn short_notes_stay_open_and_valid_notes_are_saved() {
        let (mut editor, events) = editor("e\u{301}x");
        editor.handle_key_event(KeyEvent::from(KeyCode::Enter));
        assert!(!editor.is_complete());
        editor.handle_key_event(KeyEvent::from(KeyCode::Char('y')));
        editor.handle_key_event(KeyEvent::from(KeyCode::Enter));
        assert_eq!(editor.completion(), Some(ViewCompletion::Accepted));
        assert_eq!(*events.lock().unwrap(), [EditorEvent::Changed("e\u{301}xy".into()), EditorEvent::Save("e\u{301}xy".into())]);
    }

    #[test]
    fn keep_and_cancel_report_distinct_outcomes() {
        let (mut kept, keep_events) = editor("unfinished");
        kept.handle_key_event(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(*keep_events.lock().unwrap(), [EditorEvent::Keep("unfinished".into())]);
        let (mut cancelled, cancel_events) = editor("unfinished");
        assert!(matches!(cancelled.on_ctrl_c(), CancellationEvent::Handled));
        assert_eq!(*cancel_events.lock().unwrap(), [EditorEvent::Cancel]);
    }

    #[test]
    fn long_quotation_scrolls_and_small_dialogs_remain_renderable() {
        let (mut editor, _) = editor("observation");
        assert_eq!(editor.centered_area(Rect::new(2, 3, 100, 30)), Some(Rect::new(12, 4, 80, 28)));
        assert_eq!(crate::bottom_pane::CenteredView(&editor).area(Rect::new(2, 3, 100, 30)), Rect::new(12, 4, 80, 28));
        editor.quotation = (0..20).map(|n| format!("quote line {n}")).collect::<Vec<_>>().join("\n");
        let area = Rect::new(0, 0, 40, 15);
        editor.render(area, &mut Buffer::empty(area));
        assert_eq!(editor.quote_rows.get(), 6);
        editor.handle_key_event(KeyEvent::new(KeyCode::PageDown, KeyModifiers::ALT));
        assert_eq!(editor.quote_offset.get(), 6);
        editor.handle_key_event(KeyEvent::new(KeyCode::PageUp, KeyModifiers::ALT));
        assert_eq!(editor.quote_offset.get(), 0);
        let quote = editor.quote_area.get();
        assert!(editor.handle_mouse_event(MouseEvent { kind: MouseEventKind::ScrollDown, column: quote.x, row: quote.y, modifiers: KeyModifiers::NONE }));
        assert_eq!(editor.quote_offset.get(), 1);
        assert!(editor.handle_mouse_event(MouseEvent { kind: MouseEventKind::ScrollDown, column: area.right(), row: area.bottom(), modifiers: KeyModifiers::NONE }));
        assert_eq!(editor.quote_offset.get(), 1);
        let tiny = Rect::new(0, 0, 4, 3);
        editor.render(tiny, &mut Buffer::empty(tiny));
        assert!(editor.cursor_pos(tiny).is_none());
    }
}
