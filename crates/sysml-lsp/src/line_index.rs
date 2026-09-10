//! Byte-offset ↔ LSP `Position` (UTF-16 line/character) conversion.

use lsp_types::{Position, Range};
use sysml_syntax::{TextRange, TextSize};

/// Where each line of a document begins.
///
/// The client counts UTF-16 code units from the start of a line and the
/// model counts bytes from the start of the file, so every crossing
/// between them goes through this.
pub struct LineIndex {
    /// byte offset of each line start
    starts: Vec<usize>,
}

impl LineIndex {
    /// Index `text` once, for as long as it does not change.
    pub fn new(text: &str) -> LineIndex {
        let mut starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        LineIndex { starts }
    }

    /// The line and UTF-16 column a byte offset falls on.
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

    /// The same, for both ends of a range.
    pub fn range(&self, text: &str, range: TextRange) -> Range {
        Range::new(
            self.position(text, range.start()),
            self.position(text, range.end()),
        )
    }

    /// And back: the byte offset a client's position names, or nothing
    /// where it names somewhere the document does not reach.
    pub fn offset(&self, text: &str, position: Position) -> Option<TextSize> {
        let line_start = *self.starts.get(position.line as usize)?;
        // the line's own text, without the break that ends it: a
        // character column past the end of a line belongs to the end of
        // that line, not to the start of the next one
        let line_end = self
            .starts
            .get(position.line as usize + 1)
            .map_or(text.len(), |&next| {
                let end = next - 1;
                if text[..end].ends_with('\r') {
                    end - 1
                } else {
                    end
                }
            });
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

    #[test]
    fn a_column_past_a_line_is_the_end_of_that_line_not_the_next() {
        // hovering past the end of `part x :` used to answer about
        // whatever the line below began with
        let text = "part x :\nA;\n";
        let index = LineIndex::new(text);
        assert_eq!(
            index.offset(text, Position::new(0, 99)).map(usize::from),
            Some(8)
        );
        // the break itself is not part of the line either way it is written
        let text = "part x :\r\nA;\r\n";
        let index = LineIndex::new(text);
        assert_eq!(
            index.offset(text, Position::new(0, 99)).map(usize::from),
            Some(8)
        );
        assert_eq!(
            index.offset(text, Position::new(1, 99)).map(usize::from),
            Some(12)
        );
    }
}
