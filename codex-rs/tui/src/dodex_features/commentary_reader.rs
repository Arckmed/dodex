//! Saved assistant-source reader using Codex's existing transcript selection and wrapping.

use std::{cell::{Cell, RefCell}, sync::Arc};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect, widgets::{Block, Borders, Paragraph, StatefulWidgetRef, Widget}};
use crate::bottom_pane::{BottomPaneView, CancellationEvent, TextArea, TextAreaState, ViewCompletion, ViewPresentation};
use crate::history_cell::{AgentMarkdownCell, HistoryCell, HistoryRenderMode};
use crate::keymap::RuntimeKeymap;
use crate::render::renderable::Renderable;
use crate::transcript_view::TranscriptView;
use super::commentary::Source;

#[derive(Clone, Debug)]
pub(crate) enum ReaderEvent { Close, Quote { source: Source, quotation: String } }

pub(crate) struct CommentaryReader {
    source: Source,
    cells: Vec<Arc<dyn HistoryCell>>,
    view: RefCell<TranscriptView>,
    automatic: bool,
    content_area: Cell<Rect>,
    selecting: bool,
    quote_editor: TextArea,
    quote_editor_state: RefCell<TextAreaState>,
    editing_quote: bool,
    complete: bool,
    notice: Option<String>,
    notify: Box<dyn FnMut(ReaderEvent)>,
}

impl CommentaryReader {
    pub(crate) fn set_passages(&mut self, passages: Vec<super::commentary_highlights::Passage>) { self.view.borrow_mut().set_commentary_passages(passages); }
    pub(crate) fn new(source: Source, text: String, cwd: &std::path::Path, keymap: &RuntimeKeymap, automatic: bool, notify: Box<dyn FnMut(ReaderEvent)>) -> Self {
        let mut quote_editor = TextArea::new();
        quote_editor.set_text_clearing_elements(&text);
        quote_editor.set_keymap_bindings(keymap);
        let cell = AgentMarkdownCell::new_with_inline_visualizations(text, cwd, None).with_source_item_id(Some(source.entry_id.clone())).with_source_turn_id(source.turn_id.clone());
        let cells: Vec<Arc<dyn HistoryCell>> = vec![Arc::new(cell)];
        let mut view = TranscriptView::default();
        view.set_presentation(true, HistoryRenderMode::Rich);
        view.set_keymap_bindings(keymap);
        view.copy_on_select = false;
        view.primary_selection = false;
        view.jump_to_beginning(&cells);
        Self { source, cells, view: RefCell::new(view), automatic, content_area: Cell::new(Rect::default()), selecting: false, quote_editor, quote_editor_state: RefCell::new(TextAreaState::default()), editing_quote: false, complete: false, notice: None, notify }
    }
    fn accept_selection(&mut self) {
        let selection = self.view.borrow_mut().selected_assistant();
        match selection {
            Ok(selection) => {
                let mut source = self.source.clone();
                source.display_offset = Some(selection.display_range.start);
                source.display_row = Some(selection.display_row);
                self.complete = true;
                (self.notify)(ReaderEvent::Quote { source, quotation: selection.quotation });
            }
            Err(error) => self.notice = Some(format!("Select a passage first ({error:?}).")),
        }
    }
}

impl BottomPaneView for CommentaryReader {
    fn presentation(&self) -> ViewPresentation { ViewPresentation::Centered }
    fn centered_area(&self, viewport: Rect) -> Option<Rect> {
        let width = ((u32::from(viewport.width) * 80 / 100) as u16).min(viewport.width.saturating_sub(2));
        Some(Rect::new(viewport.x + (viewport.width - width) / 2, viewport.y.saturating_add(1), width, viewport.height.saturating_sub(2)))
    }
    fn accepts_input_when_disconnected(&self) -> bool { true }
    fn keymap_contexts(&self) -> crate::keymap::KeymapContextSet {
        if self.editing_quote { crate::keymap::KeymapContextSet::new(self.quote_editor.keymap_context()) } else { crate::keymap::KeymapContextSet::default() }
    }
    fn handle_key_event(&mut self, key: KeyEvent) {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) { return; }
        if self.editing_quote {
            if key.code == KeyCode::Enter && key.modifiers.is_empty() {
                let quotation = self.quote_editor.text().trim();
                if quotation.is_empty() { self.notice = Some("Choose a nonempty quotation.".into()); return; }
                if quotation.encode_utf16().count() > 50_000 { self.notice = Some("The quotation must be at most 50,000 UTF-16 code units.".into()); return; }
                let mut source = self.source.clone(); source.display_offset = None; source.display_row = Some(0);
                self.complete = true;
                (self.notify)(ReaderEvent::Quote { source, quotation: quotation.to_owned() });
            } else if key.code == KeyCode::Enter && key.modifiers == KeyModifiers::SHIFT { self.quote_editor.insert_str("\n"); }
            else { self.quote_editor.input(key); }
            return;
        }
        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::ALT { self.accept_selection(); }
        else if key.code == KeyCode::Char('q') && key.modifiers.is_empty() { self.editing_quote = true; self.selecting = false; self.notice = None; }
        else { self.view.borrow_mut().handle_key(key, &self.cells); }
    }
    fn handle_mouse_event(&mut self, event: MouseEvent) -> bool {
        if self.editing_quote {
            let scrolled = self.quote_editor.scroll_mouse(event, &mut self.quote_editor_state.borrow_mut());
            if !scrolled { self.quote_editor.handle_mouse(event, *self.quote_editor_state.borrow()); }
            return true;
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left) { self.selecting = self.content_area.get().contains((event.column, event.row).into()); }
        self.view.borrow_mut().handle_mouse(event, &self.cells);
        let released = event.kind == MouseEventKind::Up(MouseButton::Left);
        let selected = self.automatic && released && self.selecting && self.view.borrow_mut().selected_assistant().is_ok();
        if released { self.selecting = false; }
        if selected { self.accept_selection(); }
        true
    }
    fn handle_paste(&mut self, text: String) -> bool { if !self.editing_quote || text.is_empty() { return false; } self.quote_editor.insert_str(&text); true }
    fn on_ctrl_c(&mut self) -> CancellationEvent { if !self.complete { self.complete = true; (self.notify)(ReaderEvent::Close); } CancellationEvent::Handled }
    fn is_complete(&self) -> bool { self.complete }
    fn completion(&self) -> Option<ViewCompletion> { self.complete.then_some(ViewCompletion::Accepted) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn keyboard_quotation_trims_source_and_accepts_a_single_character() {
        let source = Source { entry_id: "item".into(), turn_id: Some("turn".into()), timestamp: None, display_offset: Some(9), display_row: Some(2), roadmap: None };
        let events = Arc::new(Mutex::new(Vec::new())); let sink = Arc::clone(&events);
        let mut reader = CommentaryReader::new(source, "abcd".into(), std::path::Path::new("."), &RuntimeKeymap::defaults(), false, Box::new(move |event| sink.lock().unwrap().push(event)));
        reader.handle_key_event(KeyEvent::from(KeyCode::Char('q')));
        assert!(reader.editing_quote);
        reader.handle_key_event(KeyEvent::from(KeyCode::Home));
        for _ in 0..3 { reader.handle_key_event(KeyEvent::from(KeyCode::Delete)); }
        reader.handle_key_event(KeyEvent::from(KeyCode::Enter));
        let events = events.lock().unwrap();
        let ReaderEvent::Quote { source, quotation } = &events[0] else { panic!("quotation expected"); };
        assert_eq!(quotation, "d");
        assert_eq!(source.display_offset, None);
        assert_eq!(source.display_row, Some(0));
    }

    #[test]
    fn source_reader_drag_returns_the_saved_identity_and_quotation() {
        let source = Source { entry_id: "item".into(), turn_id: Some("turn".into()), timestamp: None, display_offset: None, display_row: None, roadmap: None };
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let mut reader = CommentaryReader::new(source, "hello reader".into(), std::path::Path::new("."), &RuntimeKeymap::defaults(), false, Box::new(move |event| sink.lock().unwrap().push(event)));
        let area = Rect::new(0, 0, 50, 12);
        let mut buffer = Buffer::empty(area);
        reader.render(area, &mut buffer);
        let row = (1..10).find(|row| (1..45).map(|col| buffer[(col, *row)].symbol()).collect::<String>().contains("hello reader")).expect("source text rendered");
        let col = (1..40).find(|col| buffer[(*col, row)].symbol() == "h").unwrap();
        for (kind, column) in [(MouseEventKind::Down(MouseButton::Left), col), (MouseEventKind::Drag(MouseButton::Left), col + 5), (MouseEventKind::Up(MouseButton::Left), col + 5)] {
            reader.handle_mouse_event(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE });
        }
        reader.automatic = true;
        reader.handle_mouse_event(MouseEvent { kind: MouseEventKind::Up(MouseButton::Left), column: 0, row: 0, modifiers: KeyModifiers::NONE });
        assert!(!reader.is_complete(), "an unrelated mouse release must not reuse an old selection");
        reader.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT));
        let events = events.lock().unwrap();
        let ReaderEvent::Quote { source, quotation } = &events[0] else { panic!("expected quotation"); };
        assert_eq!(source.entry_id, "item");
        assert_eq!(source.turn_id.as_deref(), Some("turn"));
        assert_eq!(quotation, "hello");
        assert!(source.display_offset.is_some());
    }
}

impl Renderable for CommentaryReader {
    fn desired_height(&self, _width: u16) -> u16 { 24 }
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        Block::default().borders(Borders::ALL).title(if self.editing_quote { " Choose quotation (trim this text to the passage you want) ".to_owned() } else { format!(" Assistant message {} ", self.source.entry_id) }).render(area, buffer);
        let inner = area.inner(ratatui::layout::Margin::new(1, 1));
        if inner.is_empty() { self.content_area.set(Rect::default()); return; }
        let content = Rect { height: inner.height.saturating_sub(1), ..inner };
        self.content_area.set(content);
        if self.editing_quote { StatefulWidgetRef::render_ref(&(&self.quote_editor), content, buffer, &mut self.quote_editor_state.borrow_mut()); }
        else { self.view.borrow_mut().render(content, buffer, &self.cells); }
        Paragraph::new(self.notice.as_deref().unwrap_or(if self.editing_quote { "Enter accept quotation · Shift+Enter newline · Esc cancel" } else { "Select then Alt+C · Q choose/trim quotation · scroll to read · Esc close" })).render(Rect::new(inner.x, inner.bottom() - 1, inner.width, 1), buffer);
    }
    fn cursor_pos(&self, area: Rect) -> Option<(u16,u16)> {
        if !self.editing_quote { return None; }
        let inner = area.inner(ratatui::layout::Margin::new(1, 1));
        self.quote_editor.cursor_pos_with_state(Rect { height: inner.height.saturating_sub(1), ..inner }, *self.quote_editor_state.borrow())
    }
}
