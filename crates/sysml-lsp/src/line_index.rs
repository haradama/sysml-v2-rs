//! Byte-offset ↔ LSP `Position` (UTF-16 line/character) conversion.

use lsp_types::{Position, Range};
use sysml_syntax::{TextRange, TextSize};

pub struct LineIndex {
    /// byte offset of each line start
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> LineIndex {
        let mut starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        LineIndex { starts }
    }

    pub fn position(&self, text: &str, offset: TextSize) -> Position {
        let offset = usize::from(offset).min(text.len());
        let line = self
            .starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1);
        let col_utf16: usize = text[self.starts[line]..offset]
            .chars()
            .map(char::len_utf16)
            .sum();
        Position::new(line as u32, col_utf16 as u32)
    }

    pub fn range(&self, text: &str, range: TextRange) -> Range {
        Range::new(
            self.position(text, range.start()),
            self.position(text, range.end()),
        )
    }

    pub fn offset(&self, text: &str, position: Position) -> Option<TextSize> {
        let line_start = *self.starts.get(position.line as usize)?;
        let line_end = self
            .starts
            .get(position.line as usize + 1)
            .copied()
            .unwrap_or(text.len());
        let mut utf16 = 0u32;
        for (i, c) in text[line_start..line_end].char_indices() {
            if utf16 >= position.character {
                return Some(TextSize::from((line_start + i) as u32));
            }
            utf16 += c.len_utf16() as u32;
        }
        Some(TextSize::from(line_end.min(text.len()) as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_positions() {
        let text = "abc\ndé£f\nxyz";
        let index = LineIndex::new(text);
        for (line, character) in [(0, 0), (0, 3), (1, 0), (1, 2), (2, 1)] {
            let position = Position::new(line, character);
            let offset = index.offset(text, position).unwrap();
            assert_eq!(index.position(text, offset), position);
        }
        // clamps past end of line
        assert_eq!(
            index.offset(text, Position::new(2, 99)).map(usize::from),
            Some(text.len())
        );
    }
}
