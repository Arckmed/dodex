//! Retain hidden reasoning between activity calls without breaking their compact group.
//!
//! Positions count preceding calls. Prepending history shifts those positions while sharing the
//! original immutable detail cells, so expanded output keeps its chronological source content.
//! Raw projection follows each cell's contract, retaining terminal input while hiding reasoning.

use super::HistoryCell;
use super::HistoryRenderMode;
use crate::terminal_hyperlinks::HyperlinkLine;
use crate::terminal_hyperlinks::plain_hyperlink_lines;
use std::sync::Arc;

#[derive(Debug, Default, Clone)]
pub(crate) struct ActivityDetails {
    entries: Vec<(usize, Arc<dyn HistoryCell>)>,
}

impl ActivityDetails {
    pub(crate) fn reasoning_lines(&self, width: u16, mode: HistoryRenderMode) -> Vec<HyperlinkLine> {
        let mut result = Vec::new();
        for (_, cell) in &self.entries {
            if cell.as_any().is::<super::ReasoningSummaryCell>() {
                if !result.is_empty() { result.push(HyperlinkLine::from("")); }
                result.extend(super::thought_filtered_lines(cell.as_ref(), width, true, true, mode, || cell.transcript_hyperlink_lines(width)));
            }
        }
        result
    }
    pub(crate) fn push(&mut self, after_calls: usize, cell: Arc<dyn HistoryCell>) {
        self.entries.push((after_calls, cell));
    }

    pub(crate) fn prepend(&mut self, mut older: Self, older_calls: usize) {
        for (position, _) in &mut self.entries {
            *position += older_calls;
        }
        older.entries.append(&mut self.entries);
        *self = older;
    }

    pub(crate) fn lines_after(
        &self,
        after_calls: usize,
        width: u16,
        mode: HistoryRenderMode,
    ) -> Vec<HyperlinkLine> {
        self.lines_after_with_thoughts(after_calls, width, mode, mode == HistoryRenderMode::Rich)
    }

    pub(crate) fn lines_after_with_thoughts(
        &self, after_calls: usize, width: u16, mode: HistoryRenderMode, thoughts_visible: bool,
    ) -> Vec<HyperlinkLine> {
        self.entries
            .iter()
            .filter(|(position, _)| *position == after_calls)
            .filter(|(_, cell)| thoughts_visible || !cell.as_any().is::<super::ReasoningSummaryCell>())
            .flat_map(|(_, cell)| {
                let mut lines = super::thought_filtered_lines(cell.as_ref(), width, thoughts_visible, true, mode, || match mode {
                    HistoryRenderMode::Rich => cell.transcript_hyperlink_lines(width),
                    HistoryRenderMode::Raw => plain_hyperlink_lines(cell.raw_lines()),
                });
                if mode == HistoryRenderMode::Rich && !lines.is_empty() {
                    lines.insert(/*index*/ 0, HyperlinkLine::from(""));
                }
                lines
            })
            .collect()
    }
}
