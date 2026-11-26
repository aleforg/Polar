use objc2_foundation::NSRange;

/// Translates Markdown byte offsets to TextKit character ranges.
pub(super) struct Offsets(Vec<(usize, usize)>);

impl Offsets {
    pub(super) fn new(text: &str) -> Self {
        let mut points = Vec::new();
        if !text.is_ascii() {
            let mut extra_bytes = 0;
            for (byte, ch) in text.char_indices().filter(|(_, ch)| !ch.is_ascii()) {
                extra_bytes += ch.len_utf8() - ch.len_utf16();
                let byte_end = byte + ch.len_utf8();
                points.push((byte_end, byte_end - extra_bytes));
            }
        }
        Self(points)
    }

    pub(super) fn utf16(&self, byte: usize) -> usize {
        let next = self.0.partition_point(|&(byte_end, _)| byte_end <= byte);
        next.checked_sub(1).map_or(byte, |index| {
            let (byte_end, utf16_end) = self.0[index];
            byte - byte_end + utf16_end
        })
    }

    pub(super) fn byte(&self, text: &str, utf16: usize) -> usize {
        let next = self.0.partition_point(|&(_, utf16_end)| utf16_end <= utf16);
        let mut byte = next
            .checked_sub(1)
            .map_or(utf16, |index| {
                let (byte_end, utf16_end) = self.0[index];
                byte_end.saturating_add(utf16 - utf16_end)
            })
            .min(text.len());
        // An offset inside a surrogate pair maps to the start of that character.
        while !text.is_char_boundary(byte) {
            byte -= 1;
        }
        byte
    }

    pub(super) fn range(&self, start: usize, end: usize) -> NSRange {
        let start = self.utf16(start);
        NSRange::new(start, self.utf16(end) - start)
    }
}

#[cfg(test)]
mod tests {
    use super::Offsets;

    #[test]
    fn maps_character_boundaries_and_rounds_inside_surrogate_pairs() {
        for text in ["", "ASCII", "aé🐻è中🗺 z", "🏔🏠🥝", "èaé"] {
            let offsets = Offsets::new(text);
            let mut utf16 = 0;
            for (byte, ch) in text.char_indices() {
                assert_eq!(offsets.utf16(byte), utf16, "{text:?} at byte {byte}");
                for unit in utf16..utf16 + ch.len_utf16() {
                    assert_eq!(offsets.byte(text, unit), byte, "{text:?} at unit {unit}");
                }
                utf16 += ch.len_utf16();
            }
            assert_eq!(offsets.utf16(text.len()), utf16, "{text:?} end");
            assert_eq!(offsets.byte(text, utf16 + 1), text.len(), "{text:?} end");
        }
    }
}
