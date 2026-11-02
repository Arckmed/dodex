//! Personal passage backgrounds, painted before transient selection and Find decoration.

use super::*;
use std::{collections::HashMap, ops::Range};
use crate::dodex_features::commentary_highlights::{Passage, locate};

#[derive(Default)]
pub(super) struct PassageHighlights {
    passages: Vec<Passage>,
    ranges: HashMap<usize, (Arc<TextLayout>, Vec<(Range<usize>, f64)>)>,
}

impl TranscriptView {
    pub(crate) fn set_commentary_passages(&mut self, passages: Vec<Passage>) {
        if self.commentary.passages != passages {
            self.commentary.passages = passages;
            self.commentary.ranges.clear();
        }
    }

    pub(super) fn render_commentary(&mut self, cells: &[Arc<dyn HistoryCell>], buffer: &mut Buffer) {
        if self.commentary.passages.is_empty() { return; }
        let light = crate::terminal_palette::default_bg().is_some_and(crate::color::is_light);
        let level = crate::terminal_palette::effective_stdout_color_level();
        let mut used = std::collections::HashSet::new();
        for (y, visible) in self.visible.iter().enumerate() {
            let Some(cell) = cells.get(visible.index).filter(|cell| EntryKey::cell(cell) == visible.key) else { continue; };
            let Some(assistant) = cell.as_any().downcast_ref::<crate::history_cell::AgentMarkdownCell>() else { continue; };
            let key = Arc::as_ptr(&visible.layout) as usize;
            used.insert(key);
            let (_, ranges) = self.commentary.ranges.entry(key).or_insert_with(|| {
                let ranges = self.commentary.passages.iter().filter(|passage| {
                    Some(passage.item_id.as_str()) == assistant.source_item_id() && passage.turn_id.as_deref() == assistant.source_turn_id()
                }).filter_map(|passage| locate(visible.layout.text(), passage, |offset| visible.layout.row_for_offset(offset)).map(|range| (range, passage.hue))).collect();
                (Arc::clone(&visible.layout), ranges)
            });
            let area = Rect::new(self.area.x, self.area.y + y as u16, self.area.width, 1);
            // Passage order is creation order; a newer annotation overwrites an overlapping background.
            for (range, hue) in ranges {
                let color = crate::dodex_features::commentary_colors::passage_color(*hue, light, level);
                visible.layout.background(range.clone(), area, buffer, visible.row, color);
            }
        }
        self.commentary.ranges.retain(|key, _| used.contains(key));
    }
}
