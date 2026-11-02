//! Explicit batch review with an independent final-consideration editor.

use std::cell::{Cell, RefCell};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect, widgets::{Block, Borders, Paragraph, StatefulWidgetRef, Widget}};
use crate::bottom_pane::{BottomPaneView, CancellationEvent, TextArea, TextAreaState, ViewCompletion, ViewPresentation};
use crate::keymap::{KeymapContextSet, RuntimeKeymap};
use crate::render::renderable::Renderable;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReviewEvent { Changed(String), Close(String), Send(String) }

pub(crate) struct CommentaryReview {
    preview: String,
    editor: TextArea,
    editor_state: RefCell<TextAreaState>,
    editor_focus: bool,
    locked: bool,
    scroll: Cell<usize>,
    preview_rows: Cell<usize>,
    preview_area: Cell<Rect>,
    editor_area: Cell<Rect>,
    complete: bool,
    notify: Box<dyn FnMut(ReviewEvent)>,
}

impl CommentaryReview {
    pub(crate) fn new(preview: String, final_note: &str, locked: bool, keymap: &RuntimeKeymap, notify: Box<dyn FnMut(ReviewEvent)>) -> Self {
        let mut editor = TextArea::new();
        editor.set_text_clearing_elements(final_note);
        editor.set_cursor(final_note.len());
        editor.set_keymap_bindings(keymap);
        Self { preview, editor, editor_state: RefCell::new(TextAreaState::default()), editor_focus: false, locked, scroll: Cell::new(0), preview_rows: Cell::new(0), preview_area: Cell::new(Rect::default()), editor_area: Cell::new(Rect::default()), complete: false, notify }
    }
    fn finish(&mut self, send: bool) {
        if self.complete { return; }
        self.complete = true;
        let note = self.editor.text().to_owned();
        (self.notify)(if send { ReviewEvent::Send(note) } else { ReviewEvent::Close(note) });
    }
    fn scroll_by(&self, delta: i32) {
        let max = self.preview_rows.get().saturating_sub(usize::from(self.preview_area.get().height));
        self.scroll.set((self.scroll.get() as i64 + i64::from(delta)).clamp(0, max as i64) as usize);
    }
    fn areas(&self, area: Rect) -> (Rect, Rect) {
        let inner = area.inner(ratatui::layout::Margin::new(1, 1));
        let height = self.editor.desired_height(inner.width.max(1)).clamp(3, 8).min(inner.height.saturating_sub(4));
        let preview = Rect { height: inner.height.saturating_sub(height + 2), ..inner };
        let input = Rect::new(inner.x, preview.bottom().saturating_add(1), inner.width, height).intersection(inner);
        (preview, input)
    }
}

impl BottomPaneView for CommentaryReview {
    fn presentation(&self) -> ViewPresentation { ViewPresentation::Centered }
    fn centered_area(&self, viewport: Rect) -> Option<Rect> {
        let width = ((u32::from(viewport.width) * 80 / 100) as u16).min(viewport.width.saturating_sub(2));
        Some(Rect::new(viewport.x + (viewport.width - width) / 2, viewport.y.saturating_add(1), width, viewport.height.saturating_sub(2)))
    }
    fn accepts_input_when_disconnected(&self) -> bool { true }
    fn keymap_contexts(&self) -> KeymapContextSet { if self.editor_focus { KeymapContextSet::new(self.editor.keymap_context()) } else { KeymapContextSet::default() } }
    fn handle_key_event(&mut self, key: KeyEvent) {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) || self.complete { return; }
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('s') | KeyCode::Enter) { self.finish(true); return; }
        if key.code == KeyCode::Tab || key.code == KeyCode::BackTab { self.editor_focus = !self.editor_focus && !self.locked; return; }
        if self.editor_focus && !self.locked {
            let before = self.editor.text().to_owned();
            if key.code == KeyCode::Enter { self.editor.insert_str("\n"); } else { self.editor.input(key); }
            if self.editor.text() != before { (self.notify)(ReviewEvent::Changed(self.editor.text().to_owned())); }
        } else {
            match key.code {
                KeyCode::Char('s') => self.finish(true),
                KeyCode::Up => self.scroll_by(-1), KeyCode::Down => self.scroll_by(1),
                KeyCode::PageUp => self.scroll_by(-i32::from(self.preview_area.get().height.max(1))),
                KeyCode::PageDown => self.scroll_by(i32::from(self.preview_area.get().height.max(1))),
                KeyCode::Home => self.scroll.set(0), KeyCode::End => self.scroll.set(self.preview_rows.get().saturating_sub(usize::from(self.preview_area.get().height))),
                _ => {}
            }
        }
    }
    fn handle_paste(&mut self, text: String) -> bool {
        if !self.editor_focus || self.locked || self.complete || text.is_empty() { return false; }
        self.editor.insert_str(&text); (self.notify)(ReviewEvent::Changed(self.editor.text().to_owned())); true
    }
    fn handle_mouse_event(&mut self, event: MouseEvent) -> bool {
        let position = (event.column, event.row).into();
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) { self.editor_focus = !self.locked && self.editor_area.get().contains(position); }
        if self.editor_area.get().contains(position) && !self.locked {
            let scrolled = self.editor.scroll_mouse(event, &mut self.editor_state.borrow_mut());
            if !scrolled { self.editor.handle_mouse(event, *self.editor_state.borrow()); }
        } else if self.preview_area.get().contains(position) {
            match event.kind { MouseEventKind::ScrollUp => self.scroll_by(-1), MouseEventKind::ScrollDown => self.scroll_by(1), _ => {} }
        }
        true
    }
    fn on_ctrl_c(&mut self) -> CancellationEvent { self.finish(false); CancellationEvent::Handled }
    fn is_complete(&self) -> bool { self.complete }
    fn completion(&self) -> Option<ViewCompletion> { self.complete.then_some(ViewCompletion::Accepted) }
}

impl Renderable for CommentaryReview {
    fn desired_height(&self, _width: u16) -> u16 { 24 }
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        Block::default().borders(Borders::ALL).title(" Review batch ").render(area, buffer);
        if area.width < 8 || area.height < 12 {
            self.preview_area.set(Rect::default()); self.editor_area.set(Rect::default());
            Paragraph::new("Enlarge the terminal to review. Esc keeps draft.").render(area.inner(ratatui::layout::Margin::new(1, 1)), buffer); return;
        }
        let (preview, input) = self.areas(area);
        self.preview_area.set(preview); self.editor_area.set(input);
        let lines = crate::markdown::render_markdown_for_viewport(&self.preview, preview.width).into_iter().map(|line| line.line).collect::<Vec<_>>();
        self.preview_rows.set(lines.len());
        self.scroll_by(0);
        Paragraph::new(lines.into_iter().skip(self.scroll.get()).take(usize::from(preview.height)).collect::<Vec<_>>()).render(preview, buffer);
        Paragraph::new(if self.locked { "Final consideration (original batch; read-only)" } else if self.editor_focus { "Final consideration [editing]" } else { "Final consideration · Tab or click to edit" }).render(Rect::new(input.x, input.y.saturating_sub(1), input.width, 1), buffer);
        StatefulWidgetRef::render_ref(&(&self.editor), input, buffer, &mut self.editor_state.borrow_mut());
        Paragraph::new("Tab focus · S from preview / Ctrl+S send · Esc keeps draft").render(Rect::new(area.x + 1, area.bottom() - 2, area.width - 2, 1), buffer);
    }
    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        if !self.editor_focus || self.locked || area.width < 8 || area.height < 12 { return None; }
        let (_, input) = self.areas(area);
        self.editor.cursor_pos_with_state(input, *self.editor_state.borrow())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn typing_s_in_final_field_does_not_send_and_ctrl_s_returns_exact_text() {
        let events = Arc::new(Mutex::new(Vec::new())); let sink = Arc::clone(&events);
        let mut view = CommentaryReview::new("## Quote\n\nObservation".into(), "ending", false, &RuntimeKeymap::defaults(), Box::new(move |event| sink.lock().unwrap().push(event)));
        view.handle_key_event(KeyEvent::from(KeyCode::Tab));
        view.handle_key_event(KeyEvent::from(KeyCode::Char('s')));
        assert!(!view.is_complete());
        view.handle_key_event(KeyEvent::from(KeyCode::Enter));
        view.handle_key_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert_eq!(events.lock().unwrap().last(), Some(&ReviewEvent::Send("endings\n".into())));
    }

    #[test]
    fn retry_keeps_final_text_read_only_and_close_preserves_it() {
        let events = Arc::new(Mutex::new(Vec::new())); let sink = Arc::clone(&events);
        let mut view = CommentaryReview::new("original batch".into(), "original ending", true, &RuntimeKeymap::defaults(), Box::new(move |event| sink.lock().unwrap().push(event)));
        view.handle_key_event(KeyEvent::from(KeyCode::Tab));
        assert!(!view.handle_paste("replacement".into()));
        view.handle_key_event(KeyEvent::from(KeyCode::Char('x')));
        let area = Rect::new(0, 0, 70, 20); view.render(area, &mut Buffer::empty(area));
        view.on_ctrl_c();
        assert_eq!(*events.lock().unwrap(), vec![ReviewEvent::Close("original ending".into())]);
    }
}
