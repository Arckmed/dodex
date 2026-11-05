//! Outline editor and pinned reader state. Storage and model context belong to the backend.
use std::cell::RefCell;
use std::path::PathBuf;
use codex_protocol::{ThreadId, outline::{OutlineDocument, OutlineState}};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent};
use ratatui::{buffer::Buffer, layout::Rect, widgets::{Block, Borders, Clear, Paragraph, StatefulWidgetRef, Widget}};
use crate::bottom_pane::{BottomPaneView, CancellationEvent, TextArea, TextAreaState, ViewCompletion, ViewPresentation};
use crate::keymap::RuntimeKeymap;
use crate::render::renderable::Renderable;
use super::commentary::{RoadmapSource, Source};
use super::commentary_reader::CommentaryReader;

#[derive(Default)]
pub(crate) struct OutlineUi {
    pub thread: Option<ThreadId>,
    pub state: OutlineState,
    pub root: PathBuf,
    pub reader: Option<CommentaryReader>,
    pub reader_key: String,
    pub area: Rect,
    pub loading: bool,
    pub refresh_again: bool,
    pub load_id: Option<uuid::Uuid>,
    pub dragging: bool,
    pub poll_scheduled: bool,
    pub automatic: bool,
}
#[derive(Clone, Debug)]
pub(crate) enum OutlineAction {
    Command(String),
    Apply(serde_json::Value),
    Refresh,
    Loaded { load_id: uuid::Uuid, result: Result<serde_json::Value, String> },
    Seed { source_id: String, text: String },
    Create { source_id: String, text: String, title: String, generation: u64 },
    Save { id: String, revision: String, text: String },
    External { id: String, revision: String, text: String },
    Keep { id: String, text: String },
    Cancel,
}
pub(crate) fn source(doc: &OutlineDocument, thread: ThreadId, root: &std::path::Path) -> Source {
    Source { entry_id: format!("outline:{}:{}",doc.id,doc.revision), turn_id: None, timestamp: doc.revisions.get(&doc.revision).map(|r| r.saved_at.clone()), display_offset: None, display_row: None,
        roadmap: Some(RoadmapSource { owner_id: thread.to_string(), roadmap_id: doc.id.clone(), title: doc.title.clone(), path: root.join(&doc.id).join("outline.md").display().to_string(), revision: doc.revision.clone() }) }
}
pub(crate) enum EditEvent { Save(String), External(String), Keep(String), Cancel }
pub(crate) struct OutlineEditor {
    title: String,
    editor: TextArea,
    state: RefCell<TextAreaState>,
    complete: bool,
    title_input: bool,
    notify: Box<dyn FnMut(EditEvent)>,
}
impl OutlineEditor {
    pub(crate) fn new(title: String, text: &str, title_input: bool, keymap: &RuntimeKeymap, notify: Box<dyn FnMut(EditEvent)>) -> Self {
        let mut editor = TextArea::new(); editor.set_keymap_bindings(keymap); editor.set_text_clearing_elements(text);
        Self { title, editor, state: RefCell::new(TextAreaState::default()), complete: false, title_input, notify }
    }
    fn input_area(area: Rect) -> Rect { let inner=area.inner(ratatui::layout::Margin::new(1,1)); Rect { height: inner.height.saturating_sub(1), ..inner } }
}
impl BottomPaneView for OutlineEditor {
    fn presentation(&self) -> ViewPresentation { ViewPresentation::Centered }
    fn centered_area(&self, viewport: Rect) -> Option<Rect> { Some(viewport) }
    fn accepts_input_when_disconnected(&self) -> bool { true }
    fn keymap_contexts(&self) -> crate::keymap::KeymapContextSet { crate::keymap::KeymapContextSet::new(self.editor.keymap_context()) }
    fn handle_key_event(&mut self, key: KeyEvent) {
        if !matches!(key.kind,KeyEventKind::Press | KeyEventKind::Repeat) || self.complete { return; }
        match (key.code,key.modifiers) {
            (KeyCode::Enter,KeyModifiers::NONE) => { self.complete=true; (self.notify)(EditEvent::Save(self.editor.text().into())); }
            (KeyCode::Enter,KeyModifiers::SHIFT) if !self.title_input => self.editor.insert_str("\n"),
            (KeyCode::Char('g'),KeyModifiers::CONTROL) if !self.title_input => { self.complete=true; (self.notify)(EditEvent::External(self.editor.text().into())); }
            (KeyCode::Char('d'),KeyModifiers::CONTROL) if !self.title_input => { self.complete=true; (self.notify)(EditEvent::Keep(self.editor.text().into())); }
            _ => { self.editor.input(key); }
        }
    }
    fn handle_mouse_event(&mut self, event: MouseEvent) -> bool {
        if !self.editor.scroll_mouse(event,&mut self.state.borrow_mut()) { self.editor.handle_mouse(event,*self.state.borrow()); } true
    }
    fn handle_paste(&mut self,text:String)->bool { self.editor.insert_str(&text); true }
    fn on_ctrl_c(&mut self)->CancellationEvent { self.complete=true; (self.notify)(EditEvent::Cancel); CancellationEvent::Handled }
    fn is_complete(&self)->bool { self.complete }
    fn completion(&self)->Option<ViewCompletion> { self.complete.then_some(ViewCompletion::Accepted) }
}
impl Renderable for OutlineEditor {
    fn desired_height(&self,_:u16)->u16 { 30 }
    fn render(&self,area:Rect,buffer:&mut Buffer) {
        Clear.render(area,buffer); Block::default().borders(Borders::ALL).title(self.title.as_str()).render(area,buffer);
        let input=Self::input_area(area);
        StatefulWidgetRef::render_ref(&(&self.editor),input,buffer,&mut self.state.borrow_mut());
        if area.height>2 { Paragraph::new(if self.title_input { "Enter create · Esc cancel" } else { "Enter save · Shift+Enter newline · Ctrl+G external editor · Ctrl+D keep draft · Esc cancel" }).render(Rect::new(input.x,area.bottom()-2,input.width,1),buffer); }
    }
    fn cursor_pos(&self,area:Rect)->Option<(u16,u16)> { self.editor.cursor_pos_with_state(Self::input_area(area),*self.state.borrow()) }
}
