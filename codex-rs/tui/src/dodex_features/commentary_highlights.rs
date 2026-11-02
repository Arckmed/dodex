//! Display-text anchors for passage backgrounds; observations do not affect highlight caching.

use std::ops::Range;
use super::commentary::{Comment, Draft};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Passage {
    pub(crate) item_id: String,
    pub(crate) turn_id: Option<String>,
    pub(crate) quotation: String,
    pub(crate) hue: f64,
    pub(crate) offset: Option<usize>,
    pub(crate) row: Option<usize>,
}

impl Passage {
    pub(crate) fn comment(comment: &Comment) -> Option<Self> { Self::new(&comment.source, &comment.quotation, comment.hue) }
    pub(crate) fn draft(draft: &Draft) -> Option<Self> { Self::new(&draft.source, &draft.quotation, draft.hue) }
    fn new(source: &super::commentary::Source, quotation: &str, hue: Option<f64>) -> Option<Self> {
        let hue = hue.filter(|hue| hue.is_finite())?;
        Some(Self { item_id: source.entry_id.clone(), turn_id: source.turn_id.clone(), quotation: quotation.into(), hue, offset: source.display_offset, row: source.display_row })
    }
}

fn normalized(text: &str) -> (String, Vec<Range<usize>>) {
    let mut result = String::new(); let mut positions = Vec::new();
    for (offset, ch) in text.char_indices() {
        if ch.is_whitespace() || ch == '\u{feff}' { continue; }
        result.push(ch);
        positions.extend(std::iter::repeat_n(offset..offset + ch.len_utf8(), ch.len_utf8()));
    }
    (result, positions)
}

pub(crate) fn locate(text: &str, passage: &Passage, row_at: impl Fn(usize) -> usize) -> Option<Range<usize>> {
    let (text, positions) = normalized(text); let (quote, _) = normalized(&passage.quotation);
    if quote.is_empty() { return None; }
    text.char_indices().filter_map(|(offset, _)| {
        if !text[offset..].starts_with(&quote) { return None; }
        let start = positions[offset].start; let end = positions[offset + quote.len() - 1].end;
        let distance = passage.offset.map_or_else(|| row_at(start).abs_diff(passage.row.unwrap_or(0)), |anchor| start.abs_diff(anchor));
        Some((distance, start..end))
    }).min_by_key(|(distance, range)| (*distance, range.start)).map(|(_, range)| range)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn passage(quote: &str, offset: Option<usize>) -> Passage { Passage { item_id: "item".into(), turn_id: Some("turn".into()), quotation: quote.into(), hue: 30.0, offset, row: None } }
    #[test]
    fn repeated_quotation_follows_saved_byte_anchor_and_survives_wrapping() {
        let text = "α hello\nworld · hello world";
        let start = text.rfind("hello").unwrap();
        assert_eq!(locate(text, &passage("hello world", Some(start)), |_| 0), Some(start..text.len()));
        let text = "α hello world · hello\nworld";
        assert_eq!(locate(text, &passage("hello world", Some(start)), |_| 0), Some(start..text.len()));
    }
    #[test]
    fn overlapping_matches_and_multibyte_characters_keep_valid_boundaries() {
        assert_eq!(locate("aaa", &passage("aa", Some(1)), |_| 0), Some(1..3));
        assert_eq!(locate("x 😀\n界 y", &passage("😀界", None), |_| 0), Some(2..10));
        assert_eq!(locate("text", &passage("missing", None), |_| 0), None);
    }
}
