use crate::ast::markers::def::MarkerDef;
use crate::ast::style::MarkupStyle;

/// Все маркеры zoll. Упорядочены от длинных к коротким для правильного приоритета.
pub const MARKERS: &[MarkerDef] = &[
    MarkerDef {
        open: "%%",
        close: "%%",
        style: MarkupStyle::COMMENT,
        multiline: true,
        track_depth: false,
    },
    MarkerDef {
        open: "$$",
        close: "$$",
        style: MarkupStyle::DISPLAY_FORMULA,
        multiline: true,
        track_depth: true,
    },
    MarkerDef {
        open: "!!!",
        close: "!!!",
        style: MarkupStyle::SPOILER_BLOCK,
        multiline: true,
        track_depth: true,
    },
    MarkerDef {
        open: "!!",
        close: "!!",
        style: MarkupStyle::SPOILER,
        multiline: false,
        track_depth: true,
    },
    MarkerDef {
        open: "//",
        close: "//",
        style: MarkupStyle::ITALIC,
        multiline: false,
        track_depth: true,
    },
    MarkerDef {
        open: "**",
        close: "**",
        style: MarkupStyle::BOLD,
        multiline: false,
        track_depth: true,
    },
    MarkerDef {
        open: "__",
        close: "__",
        style: MarkupStyle::UNDERLINE,
        multiline: false,
        track_depth: true,
    },
    MarkerDef {
        open: "''",
        close: "''",
        style: MarkupStyle::SUPERSCRIPT,
        multiline: false,
        track_depth: true,
    },
    MarkerDef {
        open: ",,",
        close: ",,",
        style: MarkupStyle::SUBSCRIPT,
        multiline: false,
        track_depth: true,
    },
    MarkerDef {
        open: "~~",
        close: "~~",
        style: MarkupStyle::STRIKETHROUGH,
        multiline: false,
        track_depth: true,
    },
    MarkerDef {
        open: "==",
        close: "==",
        style: MarkupStyle::HIGHLIGHT,
        multiline: false,
        track_depth: true,
    },
    MarkerDef {
        open: "++",
        close: "++",
        style: MarkupStyle::INSERTION,
        multiline: false,
        track_depth: true,
    },
    MarkerDef {
        open: "--",
        close: "--",
        style: MarkupStyle::DELETION,
        multiline: false,
        track_depth: true,
    },
    MarkerDef {
        open: "$",
        close: "$",
        style: MarkupStyle::FORMULA,
        multiline: false,
        track_depth: true,
    },
];
