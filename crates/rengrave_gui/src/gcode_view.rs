// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Syntax highlighting for the G-code view.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};
use std::ops::Range;

/// What a piece of a G-code line means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// A `( … )` or `; …` comment.
    Comment,
    /// `G0`: a move at rapid speed, not cutting.
    Rapid,
    /// `G1`, `G2`, or `G3`: a cutting move.
    Cut,
    /// Any other `G` word, which sets a mode.
    Modal,
    /// An `M` word: spindle, coolant, and program control.
    Machine,
    /// An `X` or `Y` word.
    Position,
    /// A `Z` word: the height of the tool.
    Height,
    /// An `I`, `J`, `K`, or `R` word, which describes an arc.
    Arc,
    /// An `F`, `S`, or `P` word: feed rate, spindle speed, or a mode parameter.
    Rate,
    /// A `#1` style variable.
    Variable,
    /// Anything else: spaces, operators, and unrecognized text.
    Plain,
}

/// One classified span of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Byte range within the line.
    pub range: Range<usize>,
    /// The meaning of the span.
    pub kind: TokenKind,
}

impl Token {
    /// Splits a line into tokens that together cover all of its text, in order.
    pub fn tokenize(line: &str) -> Vec<Self> {
        let mut tokens: Vec<Self> = Vec::new();
        let mut i = 0;
        while let Some(ch) = line[i..].chars().next() {
            let (len, kind) = match ch {
                '(' => (Self::comment_len(&line[i..]), TokenKind::Comment),
                ';' => (line.len() - i, TokenKind::Comment),
                '#' => {
                    let digits = Self::digits_len(&line[i + 1..]);
                    if digits > 0 {
                        (1 + digits, TokenKind::Variable)
                    } else {
                        (1, TokenKind::Plain)
                    }
                }
                c if c.is_ascii_alphabetic() => {
                    let value = Self::value_len(&line[i + 1..]);
                    if value > 0 {
                        let word = &line[i..=i + value];
                        (1 + value, Self::classify_word(c, &word[1..]))
                    } else {
                        (1, TokenKind::Plain)
                    }
                }
                c => (c.len_utf8(), TokenKind::Plain),
            };
            let end = i + len;
            match tokens.last_mut() {
                Some(last) if kind == TokenKind::Plain && last.kind == TokenKind::Plain => {
                    last.range.end = end;
                }
                _ => tokens.push(Self {
                    range: i..end,
                    kind,
                }),
            }
            i = end;
        }
        tokens
    }

    /// Length of the parenthesized comment at the start of `text`, allowing nested parentheses.
    /// An unclosed comment runs to the end of the line.
    fn comment_len(text: &str) -> usize {
        let mut depth = 0;
        for (i, ch) in text.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
        }
        text.len()
    }

    fn digits_len(text: &str) -> usize {
        text.bytes().take_while(u8::is_ascii_digit).count()
    }

    /// Length of the value after a word's letter: a signed number or a `#1` variable.
    fn value_len(text: &str) -> usize {
        if let Some(rest) = text.strip_prefix('#') {
            let digits = Self::digits_len(rest);
            return if digits > 0 { 1 + digits } else { 0 };
        }
        let sign = usize::from(text.starts_with(['+', '-']));
        let number = text.as_bytes()[sign..]
            .iter()
            .take_while(|b| b.is_ascii_digit() || **b == b'.')
            .count();
        if number > 0 { sign + number } else { 0 }
    }

    fn classify_word(letter: char, value: &str) -> TokenKind {
        match letter.to_ascii_uppercase() {
            'G' => match value.parse::<f64>() {
                Ok(0.0) => TokenKind::Rapid,
                Ok(n) if n == 1.0 || n == 2.0 || n == 3.0 => TokenKind::Cut,
                _ => TokenKind::Modal,
            },
            'M' => TokenKind::Machine,
            'X' | 'Y' => TokenKind::Position,
            'Z' => TokenKind::Height,
            'I' | 'J' | 'K' | 'R' => TokenKind::Arc,
            'F' | 'S' | 'P' => TokenKind::Rate,
            _ => TokenKind::Plain,
        }
    }

    /// The color of this kind of token. `plain` is the theme's normal text color.
    fn color(kind: TokenKind, dark: bool, plain: Color32) -> Color32 {
        let rgb = |dark_color: [u8; 3], light_color: [u8; 3]| {
            let [r, g, b] = if dark { dark_color } else { light_color };
            Color32::from_rgb(r, g, b)
        };
        match kind {
            TokenKind::Comment => rgb([120, 160, 120], [70, 120, 70]),
            TokenKind::Rapid => rgb([240, 150, 80], [190, 90, 10]),
            TokenKind::Cut => rgb([100, 170, 255], [20, 90, 190]),
            TokenKind::Modal => rgb([190, 150, 240], [120, 60, 180]),
            TokenKind::Machine => rgb([240, 120, 170], [180, 30, 100]),
            TokenKind::Height => rgb([90, 210, 200], [0, 125, 120]),
            TokenKind::Arc => rgb([220, 200, 110], [130, 110, 0]),
            TokenKind::Rate => rgb([210, 170, 120], [140, 90, 30]),
            TokenKind::Variable => rgb([230, 130, 110], [170, 60, 40]),
            TokenKind::Position | TokenKind::Plain => plain,
        }
    }

    /// Lays out a line with each token in its color.
    pub fn highlight(line: &str, font: &FontId, dark: bool, plain: Color32) -> LayoutJob {
        let mut job = LayoutJob::default();
        for token in Self::tokenize(line) {
            job.append(
                &line[token.range],
                0.0,
                TextFormat::simple(font.clone(), Self::color(token.kind, dark, plain)),
            );
        }
        job
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TokenKind::*;

    fn kinds(line: &str) -> Vec<(&str, TokenKind)> {
        Token::tokenize(line)
            .into_iter()
            .map(|t| (&line[t.range], t.kind))
            .collect()
    }

    #[test]
    fn motion_words_separate_rapids_from_cuts() {
        assert_eq!(kinds("G0"), [("G0", Rapid)]);
        assert_eq!(kinds("G00"), [("G00", Rapid)]);
        for cut in ["G1", "G2", "G3", "G01", "g1"] {
            assert_eq!(kinds(cut), [(cut, Cut)]);
        }
        for modal in ["G17", "G20", "G90", "G91.1"] {
            assert_eq!(kinds(modal), [(modal, Modal)]);
        }
    }

    #[test]
    fn cutting_move_with_axes() {
        assert_eq!(
            kinds("G1 X0.1405 Y-0.1013 Z-0.1755"),
            [
                ("G1", Cut),
                (" ", Plain),
                ("X0.1405", Position),
                (" ", Plain),
                ("Y-0.1013", Position),
                (" ", Plain),
                ("Z-0.1755", Height),
            ]
        );
    }

    #[test]
    fn words_may_touch_without_a_space() {
        assert_eq!(
            kinds("G1 X1.000F5.0"),
            [
                ("G1", Cut),
                (" ", Plain),
                ("X1.000", Position),
                ("F5.0", Rate)
            ]
        );
    }

    #[test]
    fn arcs_and_preamble() {
        assert_eq!(
            kinds("G3 I-1.0 J0.0 R2"),
            [
                ("G3", Cut),
                (" ", Plain),
                ("I-1.0", Arc),
                (" ", Plain),
                ("J0.0", Arc),
                (" ", Plain),
                ("R2", Arc),
            ]
        );
        assert_eq!(
            kinds("G64 P0.001 M3 S3000"),
            [
                ("G64", Modal),
                (" ", Plain),
                ("P0.001", Rate),
                (" ", Plain),
                ("M3", Machine),
                (" ", Plain),
                ("S3000", Rate),
            ]
        );
    }

    #[test]
    fn variables_as_definitions_and_values() {
        assert_eq!(
            kinds("#1 = 0.2500  ( Safe Z )"),
            [
                ("#1", Variable),
                (" = 0.2500  ", Plain),
                ("( Safe Z )", Comment),
            ]
        );
        assert_eq!(
            kinds("G0 Z#1"),
            [("G0", Rapid), (" ", Plain), ("Z#1", Height)]
        );
    }

    #[test]
    fn comments_hide_their_contents() {
        assert_eq!(kinds("(G1 X1)"), [("(G1 X1)", Comment)]);
        assert_eq!(
            kinds("G90 ; G1 X1"),
            [("G90", Modal), (" ", Plain), ("; G1 X1", Comment)]
        );
        assert_eq!(
            kinds("(a (b) c) G0"),
            [("(a (b) c)", Comment), (" ", Plain), ("G0", Rapid)]
        );
        assert_eq!(kinds("(unclosed G1"), [("(unclosed G1", Comment)]);
    }

    #[test]
    fn malformed_text_stays_plain() {
        assert_eq!(kinds("hello"), [("hello", Plain)]);
        assert_eq!(kinds("# X- G"), [("# X- G", Plain)]);
        assert_eq!(kinds("G."), [("G.", Modal)]);
        assert!(kinds("").is_empty());
    }

    #[test]
    fn tokens_cover_the_line_including_non_ascii() {
        for line in [
            "(fengrave_set fontfile \"Ünï çødé.ttf\" )",
            "é G1 ü X1 ✓",
            "G1 X1.000 Y0.000F5.0",
            "#",
            "G",
            "((",
            "G-",
        ] {
            let tokens = Token::tokenize(line);
            let mut joined = String::new();
            let mut expected_start = 0;
            for token in tokens {
                assert_eq!(token.range.start, expected_start, "{line}");
                expected_start = token.range.end;
                joined.push_str(&line[token.range]);
            }
            assert_eq!(joined, line);
        }
    }

    #[test]
    fn layout_preserves_text_and_colors_tokens() {
        let plain = Color32::from_gray(200);
        for dark in [true, false] {
            let job = Token::highlight("G1 X1 Z-2 (c)", &FontId::monospace(12.0), dark, plain);
            assert_eq!(job.text, "G1 X1 Z-2 (c)");
            let colors: Vec<Color32> = job.sections.iter().map(|s| s.format.color).collect();
            assert!(colors.contains(&plain));
            assert!(colors.contains(&Token::color(Cut, dark, plain)));
            assert!(colors.contains(&Token::color(Height, dark, plain)));
            assert!(colors.contains(&Token::color(Comment, dark, plain)));
        }
    }

    #[test]
    fn each_special_kind_has_its_own_color_in_both_themes() {
        let plain = Color32::from_gray(200);
        let special = [
            Comment, Rapid, Cut, Modal, Machine, Height, Arc, Rate, Variable,
        ];
        for dark in [true, false] {
            let mut colors: Vec<_> = special
                .iter()
                .map(|k| Token::color(*k, dark, plain).to_array())
                .collect();
            colors.sort_unstable();
            colors.dedup();
            assert_eq!(colors.len(), special.len());
        }
    }
}
