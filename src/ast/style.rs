use std::ops::BitOr;

/// Битовая маска стилей разметки.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkupStyle(pub u32);

impl MarkupStyle {
    pub const PLAIN: Self = Self(0);
    pub const BOLD: Self = Self(1 << 0);
    pub const ITALIC: Self = Self(1 << 1);
    pub const UNDERLINE: Self = Self(1 << 2);
    pub const STRIKETHROUGH: Self = Self(1 << 3);
    pub const SUPERSCRIPT: Self = Self(1 << 4);
    pub const SUBSCRIPT: Self = Self(1 << 5);
    pub const CODE: Self = Self(1 << 6);
    pub const HIGHLIGHT: Self = Self(1 << 7);
    pub const SPOILER: Self = Self(1 << 8);
    pub const SPOILER_BLOCK: Self = Self(1 << 9);
    pub const INSERTION: Self = Self(1 << 10);
    pub const DELETION: Self = Self(1 << 11);
    pub const COMMENT: Self = Self(1 << 12);
    pub const FORMULA: Self = Self(1 << 13);
    pub const DISPLAY_FORMULA: Self = Self(1 << 14);

    pub fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub fn bits(&self) -> u32 {
        self.0
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }
}

impl BitOr for MarkupStyle {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
