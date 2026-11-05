// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Comparison of G-code programs for the tests that check output against F-Engrave's.
//!
//! The V-carve paths come from sines, cosines, and arctangents, and platforms differ in the last
//! bit of those. A coordinate that is unchanged on one platform can then differ on another, so
//! that a word such as `X0.3333` is printed on one and omitted on the other, or a value that
//! lies on a rounding boundary prints one digit apart. The comparison therefore expands the
//! modal words, so that every move names its full position, and allows each coordinate to
//! differ by one unit of its last printed digit. Everything else must match exactly.

// Each test binary uses part of this module.
#![allow(
    dead_code,
    reason = "shared by test binaries that each use a part of it"
)]

use std::collections::BTreeMap;

/// A coordinate word: its value, and the number of decimals it was printed with.
type Word = (f64, usize);

#[derive(Debug)]
enum Line {
    /// Anything that is not a move: headers, spindle commands, comments.
    Text(String),
    /// A move with the position it ends at, and the words that are not modal.
    Move {
        code: String,
        position: [Word; 3],
        extra: BTreeMap<char, Word>,
        feed: Option<String>,
    },
}

impl Line {
    /// Reads `lines`, carrying X, Y, and Z from one move to the next.
    fn expand<'a>(lines: impl IntoIterator<Item = &'a str>) -> Vec<Self> {
        let mut position: [Word; 3] = [(0.0, 0); 3];
        let mut out: Vec<Self> = Vec::new();
        for line in lines {
            let mut tokens = line.split_whitespace();
            let Some(code) = tokens.next() else {
                out.push(Self::Text(String::new()));
                continue;
            };
            if !matches!(code, "G0" | "G1" | "G2" | "G3") {
                out.push(Self::Text(line.to_string()));
                continue;
            }
            let mut extra = BTreeMap::new();
            let mut feed = None;
            for token in tokens {
                let (letter, number) = token.split_at(1);
                let Ok(value) = number.parse::<f64>() else {
                    continue;
                };
                let decimals = number.split_once('.').map_or(0, |(_, d)| d.len());
                match letter {
                    "X" => position[0] = (value, decimals),
                    "Y" => position[1] = (value, decimals),
                    "Z" => position[2] = (value, decimals),
                    "F" => feed = Some(number.to_string()),
                    other => {
                        extra.insert(other.chars().next().unwrap_or('?'), (value, decimals));
                    }
                }
            }
            let this = Self::Move {
                code: code.to_string(),
                position,
                extra,
                feed,
            };
            // A move to where the tool already is, with nothing else on it, changes nothing.
            if let Self::Move {
                extra, feed: None, ..
            } = &this
                && extra.is_empty()
                && this.same_as_previous(out.last())
            {
                continue;
            }
            out.push(this);
        }
        out
    }

    fn same_as_previous(&self, previous: Option<&Self>) -> bool {
        match (self, previous) {
            (
                Self::Move { code, position, .. },
                Some(Self::Move {
                    code: previous_code,
                    position: previous_position,
                    ..
                }),
            ) => code == previous_code && Self::near(position, previous_position),
            _ => false,
        }
    }

    fn near(a: &[Word; 3], b: &[Word; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| Self::close(*x, *y))
    }

    /// Whether two printed values are the same, or one digit apart in the last place.
    fn close(a: Word, b: Word) -> bool {
        let decimals = i32::try_from(a.1.max(b.1)).unwrap_or(4);
        (a.0 - b.0).abs() <= 1.5 * 10f64.powi(-decimals)
    }

    fn matches(&self, expected: &Self) -> bool {
        match (self, expected) {
            (Self::Text(a), Self::Text(e)) => a == e,
            (
                Self::Move {
                    code,
                    position,
                    extra,
                    feed,
                },
                Self::Move {
                    code: e_code,
                    position: e_position,
                    extra: e_extra,
                    feed: e_feed,
                },
            ) => {
                code == e_code
                    && feed == e_feed
                    && Self::near(position, e_position)
                    && extra.len() == e_extra.len()
                    && extra
                        .iter()
                        .all(|(k, v)| e_extra.get(k).is_some_and(|e| Self::close(*v, *e)))
            }
            _ => false,
        }
    }
}

/// Fails unless `actual` is the program `expected`, within the tolerance described above.
pub fn assert_same_program(name: &str, actual: &[String], expected: &[&str]) {
    let actual_lines = Line::expand(actual.iter().map(String::as_str));
    let expected_lines = Line::expand(expected.iter().copied());
    for (i, (a, e)) in actual_lines.iter().zip(&expected_lines).enumerate() {
        assert!(
            a.matches(e),
            "{name}: first difference at move or line {}:\n  actual   {a:?}\n  expected {e:?}",
            i + 1
        );
    }
    assert_eq!(
        actual_lines.len(),
        expected_lines.len(),
        "{name}: program length"
    );
}
