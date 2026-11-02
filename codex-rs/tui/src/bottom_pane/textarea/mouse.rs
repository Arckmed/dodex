//! Hit testing and editable mouse selections share the textarea's rendered wrap and scroll state.

use super::*;
use crate::text_selection::SelectionUnit;
use crossterm::event::MouseButton;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use ratatui::layout::Position;
use std::sync::Arc;

#[derive(Debug)]
pub(super) struct MouseSelection {
    pending_copy: Option<(u64, Range<usize>)>,
    cut_on_copy: bool,
    primary_owner: Option<Arc<()>>,
    origin: Range<usize>,
    unit: SelectionUnit,
    dragging: bool,
    moved: bool,
}

impl TextArea {
    pub(in crate::bottom_pane) fn defer_copy(&mut self, id: u64) {
        if let Some(range) = self.mouse_selection_range()
            && let Some(selection) = &mut self.mouse_selection
        {
            selection.pending_copy = Some((id, range));
            selection.cut_on_copy = false;
        }
    }

    pub(in crate::bottom_pane) fn defer_cut(&mut self, id: u64) {
        self.defer_copy(id);
        if let Some(selection) = &mut self.mouse_selection {
            selection.cut_on_copy = true;
        }
    }

    pub(in crate::bottom_pane) fn pending_cut(&self, id: u64) -> bool {
        self.mouse_selection.as_ref().is_some_and(|selection| {
            selection.cut_on_copy
                && selection.pending_copy.as_ref().is_some_and(|pending| {
                    pending.0 == id && self.mouse_selection_range().as_ref() == Some(&pending.1)
                })
        })
    }

    /// Opted into only by the main composer; other textarea users retain their keymaps.
    pub(in crate::bottom_pane) fn composer_selection_key(&self, key: KeyEvent) -> bool {
        let shifted = key.modifiers == KeyModifiers::SHIFT;
        let word = key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        (key.code == KeyCode::Char('a') && key.modifiers == KeyModifiers::CONTROL)
            || (shifted && matches!(key.code, KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Home | KeyCode::End))
            || (word && matches!(key.code, KeyCode::Left | KeyCode::Right))
            || (key.modifiers.is_empty() && matches!(key.code, KeyCode::Esc | KeyCode::Left | KeyCode::Right) && self.mouse_selection_range().is_some())
    }

    pub(in crate::bottom_pane) fn input_composer_selection(&mut self, key: KeyEvent) -> bool {
        if !self.composer_selection_key(key) || key.kind == KeyEventKind::Release {
            return false;
        }
        self.last_click = None;
        if key.modifiers.is_empty() {
            if let Some(range) = self.mouse_selection_range() {
                let position = match key.code {
                    KeyCode::Left => range.start,
                    KeyCode::Right => range.end,
                    _ => self.cursor_pos,
                };
                self.set_cursor(position);
            }
            return true;
        }
        let select_all = key.code == KeyCode::Char('a');
        let origin = if select_all {
            0..0
        } else {
            self.mouse_selection.as_ref().map_or(
                self.cursor_pos..self.cursor_pos,
                |selection| selection.origin.clone(),
            )
        };
        let word = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('a') => self.set_cursor(self.text.len()),
            KeyCode::Left if word => self.set_cursor(self.beginning_of_previous_word()),
            KeyCode::Right if word => self.set_cursor(self.end_of_next_word()),
            KeyCode::Left => self.move_cursor_left(),
            KeyCode::Right => self.move_cursor_right(),
            KeyCode::Up => self.move_cursor_up(),
            KeyCode::Down => self.move_cursor_down(),
            KeyCode::Home => self.move_cursor_to_beginning_of_line(false),
            KeyCode::End => self.move_cursor_to_end_of_line(false),
            _ => return false,
        }
        self.mouse_selection = Some(MouseSelection {
            pending_copy: None,
            cut_on_copy: false,
            primary_owner: None,
            origin,
            unit: SelectionUnit::Character,
            dragging: false,
            moved: true,
        });
        true
    }

    pub(in crate::bottom_pane) fn retain_primary(&mut self, owner: Option<Arc<()>>) {
        if let Some(selection) = &mut self.mouse_selection {
            selection.primary_owner = owner;
        }
    }

    pub(in crate::bottom_pane) fn cancel_primary(&mut self) {
        self.retain_primary(/*owner*/ None);
    }

    pub(in crate::bottom_pane) fn finish_copy(
        &mut self,
        completion: &(u64, crate::clipboard_copy::worker::CopyResult),
        current: bool,
    ) -> Option<usize> {
        let pending = self.mouse_selection.as_mut()?.pending_copy.as_ref()?;
        if pending.0 != completion.0 {
            return None;
        }
        let (_, range) = self.mouse_selection.as_mut()?.pending_copy.take()?;
        if !current || self.mouse_selection_range().as_ref() != Some(&range) {
            return None;
        }
        let characters = self.text()[range.clone()].chars().count();
        if completion.1 == Ok(crate::clipboard_copy::CopyStatus::Confirmed) {
            if self.mouse_selection.as_ref().is_some_and(|s| s.cut_on_copy) {
                self.replace_range(range, "");
            } else {
                self.set_cursor(self.cursor());
            }
        }
        Some(characters)
    }

    pub(in crate::bottom_pane) fn contains_mouse(&self, event: MouseEvent) -> bool {
        self.rendered_area
            .get()
            .contains(Position::new(event.column, event.row))
    }

    pub(crate) fn end_mouse_drag(&mut self) {
        if let Some(selection) = &mut self.mouse_selection {
            selection.dragging = false;
        }
    }

    pub(crate) fn mouse_selection_range(&self) -> Option<Range<usize>> {
        let origin = &self.mouse_selection.as_ref()?.origin;
        let range = origin.start.min(self.cursor_pos)..origin.end.max(self.cursor_pos);
        (!range.is_empty()).then_some(range)
    }

    /// Consume events at the edges too, but only detach from the caret if the viewport moves.
    pub(crate) fn scroll_mouse(&self, event: MouseEvent, state: &mut TextAreaState) -> bool {
        let area = self.rendered_area.get();
        let Some(rows) = crate::bottom_pane::scroll_state::wheel_rows_in(event, area) else {
            return false;
        };
        let lines = self.wrapped_lines(area.width);
        let total = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        if total <= area.height {
            return false;
        }
        let scroll = state
            .scroll
            .saturating_add_signed(rows)
            .min(total - area.height);
        if scroll != state.scroll {
            state.scroll = scroll;
            state.manual_cursor = Some(self.cursor_pos);
        }
        true
    }

    pub(crate) fn handle_mouse(&mut self, event: MouseEvent, state: TextAreaState) -> bool {
        let area = self.rendered_area.get();
        let dragging = self.mouse_selection.as_ref().is_some_and(|s| s.dragging);
        if area.is_empty() {
            self.end_mouse_drag();
            return false;
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left)
                if event.modifiers.is_empty() && self.contains_mouse(event) => {}
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
                if dragging => {}
            _ => return false,
        }

        if matches!(event.kind, MouseEventKind::Up(MouseButton::Left))
            && self.mouse_selection.as_ref().is_some_and(|s| !s.moved)
        {
            self.end_mouse_drag();
            return true;
        }
        let line = {
            let lines = self.wrapped_lines(area.width);
            let row = usize::from(state.scroll)
                + usize::from(event.row.saturating_sub(area.y).min(area.height - 1));
            // Dragging beyond the viewport advances one wrapped row per event.
            let row = if event.row < area.y {
                row.saturating_sub(1)
            } else if event.row >= area.bottom() {
                row + 1
            } else {
                row
            };
            lines[row.min(lines.len() - 1)].clone()
        };
        let col = usize::from(event.column.saturating_sub(area.x).min(area.width));
        let pos = self.position_at_display_col_on_line(line.start, line.end - 1, col);
        let down = matches!(event.kind, MouseEventKind::Down(MouseButton::Left));
        let mut selection = if down {
            if self.mouse_selection.is_none() {
                self.last_click = None;
            }
            let clicks =
                crate::text_selection::click_count(&mut self.last_click, event.column, event.row);
            MouseSelection {
                pending_copy: None,
                cut_on_copy: false,
                primary_owner: None,
                origin: pos..pos,
                // Padding remains an insertion target on repeated clicks.
                unit: SelectionUnit::from_clicks(if pos == line.end - 1 { 1 } else { clicks }),
                dragging: true,
                moved: false,
            }
        } else if let Some(selection) = self.mouse_selection.take() {
            selection
        } else {
            return false;
        };
        let unit = selection.unit;
        let range = if unit == SelectionUnit::Character {
            let pos = self.clamp_pos_to_nearest_boundary(pos);
            pos..pos
        } else if unit == SelectionUnit::Word
            && let Some(element) = self.elements.iter().find(|e| e.range.contains(&pos))
        {
            element.range.clone()
        } else {
            self.expand_range_to_element_boundaries(unit.range(&self.text, pos))
        };
        self.preferred_col = None;
        self.clear_vim_replace_recovery();
        self.vim_pending = VimPending::None;
        if down {
            selection.origin = range.clone();
        }
        self.cursor_pos = if range.start < selection.origin.start {
            range.start
        } else {
            range.end
        };
        selection.moved |= !down;
        selection.dragging = !matches!(event.kind, MouseEventKind::Up(MouseButton::Left));
        self.mouse_selection = Some(selection);
        true
    }
}

#[cfg(test)]
#[path = "mouse_tests.rs"]
mod tests;
