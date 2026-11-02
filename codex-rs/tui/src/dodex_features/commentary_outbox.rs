//! Pending annotations panel. The app controller owns mutations and confirmation flows.

use std::cell::RefCell;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect, style::Stylize, text::Line, widgets::{Block, Borders, Paragraph, Widget}};
use crate::bottom_pane::{BottomPaneView, CancellationEvent, ViewCompletion, ViewPresentation};
use crate::render::renderable::Renderable;
use super::commentary::Commentary;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OutboxAction { Close, Add, Resume, DiscardDraft, Review, Edit(String), Delete(String), Source(String) }

pub(crate) struct CommentaryOutbox {
    state: Commentary,
    selected: usize,
    hits: RefCell<Vec<(Rect, usize)>>,
    source_area: RefCell<Rect>,
    complete: bool,
    notify: Box<dyn FnMut(OutboxAction)>,
}

impl CommentaryOutbox {
    pub(crate) fn new(state: Commentary, notify: Box<dyn FnMut(OutboxAction)>) -> Self {
        Self { state, selected: 0, hits: RefCell::new(Vec::new()), source_area: RefCell::new(Rect::default()), complete: false, notify }
    }
    fn finish(&mut self, action: OutboxAction) { if !self.complete { self.complete = true; (self.notify)(action); } }
    fn move_selection(&mut self, delta: i32) { self.selected = (self.selected as i64 + i64::from(delta)).clamp(0, self.state.comments.len().saturating_sub(1) as i64) as usize; }
    fn selected_action(&mut self, action: fn(String) -> OutboxAction) { if let Some(comment) = self.state.comments.get(self.selected) { self.finish(action(comment.id.clone())); } }
}

impl BottomPaneView for CommentaryOutbox {
    fn presentation(&self) -> ViewPresentation { ViewPresentation::Centered }
    fn centered_area(&self, viewport: Rect) -> Option<Rect> {
        let width = ((u32::from(viewport.width) * 65 / 100) as u16).min(viewport.width.saturating_sub(2));
        Some(Rect::new(viewport.right().saturating_sub(width).saturating_sub(1).max(viewport.x), viewport.y.saturating_add(1), width, viewport.height.saturating_sub(2)))
    }
    fn accepts_input_when_disconnected(&self) -> bool { true }
    fn handle_key_event(&mut self, key: KeyEvent) {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) { return; }
        match key.code {
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Enter | KeyCode::Char('e') => self.selected_action(OutboxAction::Edit),
            KeyCode::Delete | KeyCode::Char('d') => self.selected_action(OutboxAction::Delete),
            KeyCode::Char('v') => self.selected_action(OutboxAction::Source),
            KeyCode::Char('a') => self.finish(OutboxAction::Add),
            KeyCode::Char('r') if self.state.draft.is_some() => self.finish(OutboxAction::Resume),
            KeyCode::Char('x') if self.state.draft.is_some() => self.finish(OutboxAction::DiscardDraft),
            KeyCode::Char('s') if !self.state.comments.is_empty() || self.state.final_consideration.as_deref().is_some_and(|text| !text.trim().is_empty()) => self.finish(OutboxAction::Review),
            KeyCode::Esc => self.finish(OutboxAction::Close),
            _ => {}
        }
    }
    fn handle_mouse_event(&mut self, event: MouseEvent) -> bool {
        match event.kind {
            MouseEventKind::ScrollUp => self.move_selection(-1),
            MouseEventKind::ScrollDown => self.move_selection(1),
            MouseEventKind::Down(MouseButton::Left) => {
                let position = (event.column, event.row).into();
                let source_clicked = self.source_area.borrow().contains(position);
                if source_clicked { self.selected_action(OutboxAction::Source); }
                else {
                    let selected = self.hits.borrow().iter().find(|(area, _)| area.contains(position)).map(|(_, index)| *index);
                    if let Some(selected) = selected { self.selected = selected; }
                }
            }
            _ => {}
        }
        true
    }
    fn on_ctrl_c(&mut self) -> CancellationEvent { self.finish(OutboxAction::Close); CancellationEvent::Handled }
    fn is_complete(&self) -> bool { self.complete }
    fn completion(&self) -> Option<ViewCompletion> { self.complete.then_some(ViewCompletion::Accepted) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn final_consideration_without_comments_remains_reviewable() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let state = Commentary { final_consideration: Some("Please check the result".into()), ..Default::default() };
        let mut view = CommentaryOutbox::new(state, Box::new(move |event| sink.lock().unwrap().push(event)));
        let area = Rect::new(0, 0, 60, 18);
        let mut buffer = Buffer::empty(area);
        view.render(area, &mut buffer);
        let text = buffer.content.iter().map(|cell| cell.symbol()).collect::<String>();
        assert!(text.contains("Final consideration saved:"));
        view.handle_key_event(KeyEvent::from(KeyCode::Char('s')));
        assert_eq!(*events.lock().unwrap(), vec![OutboxAction::Review]);
        assert!(view.is_complete());
    }

    #[test]
    fn empty_outbox_cannot_send_and_tiny_view_can_close() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let mut view = CommentaryOutbox::new(Commentary::default(), Box::new(move |event| sink.lock().unwrap().push(event)));
        view.handle_key_event(KeyEvent::from(KeyCode::Char('s')));
        assert!(!view.is_complete());
        let area = Rect::new(0, 0, 3, 2);
        view.render(area, &mut Buffer::empty(area));
        view.on_ctrl_c();
        assert_eq!(*events.lock().unwrap(), vec![OutboxAction::Close]);
    }
}

fn summary(text: &str) -> String { text.split_whitespace().collect::<Vec<_>>().join(" ") }

impl Renderable for CommentaryOutbox {
    fn desired_height(&self, _width: u16) -> u16 { 20 }
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        self.hits.borrow_mut().clear();
        *self.source_area.borrow_mut() = Rect::default();
        Block::default().borders(Borders::ALL).title(" Pending comments ").render(area, buffer);
        let inner = area.inner(ratatui::layout::Margin::new(1, 1));
        if inner.is_empty() { return; }
        if inner.width < 6 || inner.height < 6 { Paragraph::new("Enlarge the terminal. Esc closes.").render(inner, buffer); return; }
        let mut lines = vec![Line::from(format!("{} pending · auto-commentary {}", self.state.comments.len(), if self.state.enabled { "on" } else { "off" })).dim()];
        if self.state.draft.is_some() { lines.push(Line::from("Unfinished draft · R resume / X discard").yellow()); }
        if let Some(text) = self.state.final_consideration.as_deref().filter(|text| !text.trim().is_empty()) { lines.push(Line::from(format!("Final consideration saved: {}", summary(text))).dim()); }
        if self.state.submission.is_some() { lines.push(Line::from("Last send not acknowledged; comments retained.").yellow()); }
        let count = usize::from(inner.height.saturating_sub(lines.len() as u16 + 6) / 2).max(1);
        let offset = self.selected.saturating_add(1).saturating_sub(count);
        if self.state.comments.is_empty() { lines.push(Line::from("No saved comments yet. A browses earlier replies.")); }
        for (index, comment) in self.state.comments.iter().enumerate().skip(offset).take(count) {
            self.hits.borrow_mut().push((Rect::new(inner.x, inner.y + lines.len() as u16, inner.width, 2), index));
            let line = Line::from(format!("{} {}. “{}”", if index == self.selected { "›" } else { " " }, index + 1, summary(&comment.quotation)));
            lines.push(if index == self.selected { line.cyan() } else { line });
            lines.push(Line::from(format!("   {}", summary(&comment.note))).dim());
        }
        if let Some(comment) = self.state.comments.get(self.selected) {
            lines.push(Line::default());
            *self.source_area.borrow_mut() = Rect::new(inner.x, inner.y + lines.len() as u16, inner.width, 1);
            lines.push(Line::from(match &comment.source.roadmap { Some(source) => format!("Roadmap {} · {} · V to view", summary(&source.title), source.revision), None => format!("Source {} · click here / V to view", comment.source.entry_id) }).cyan());
            lines.extend(comment.note.lines().take(3).map(|line| Line::from(line.to_owned())));
        }
        Paragraph::new(lines).render(Rect { height: inner.height.saturating_sub(1), ..inner }, buffer);
        Paragraph::new("↑↓ select · Enter edit · D delete · V source · A add · S review · Esc close").render(Rect::new(inner.x, inner.bottom() - 1, inner.width, 1), buffer);
    }
}
