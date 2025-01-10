//! General utilities for the CLI application.
//!
//! This module provides utility functions and types for common operations across the CLI.

use core::fmt;
use std::{collections::BTreeMap, fmt::Display};

/// A constant marker used for visual emphasis in CLI output. It's a red asterisk.
pub const MARKER: &str = "\u{001b}[1m\u{001b}[31m*\u{001b}[0m";

/// Prints a list of paths in a tree-like structure.
///
/// Takes a list of paths and a delimiter character, then outputs them in a hierarchical
/// format similar to the `tree` command, with branch indicators (├── and └──) showing the
/// relationships between path segments.
pub fn print_paths(paths: &[String], delimiter: char) {
    struct Node(BTreeMap<String, Node>);

    let mut tree: BTreeMap<String, Node> = BTreeMap::new();

    for path in paths {
        let mut current = &mut tree;
        for segment in path.split(delimiter) {
            current = &mut current
                .entry(segment.to_owned())
                .or_insert_with(|| Node(BTreeMap::new()))
                .0;
        }
    }

    fn print_level(level: &BTreeMap<String, Node>, is_last: Vec<bool>) {
        let prefix = is_last
            .iter()
            .map(|&is_last| if is_last { "    " } else { "│   " })
            .collect::<String>();

        for (i, (key, Node(subtree))) in level.iter().enumerate() {
            if key.is_empty() {
                continue;
            }

            if i + 1 != level.len() {
                println!("{}├── {}", prefix, key);
                print_level(subtree, [&is_last, [false].as_ref()].concat());
            } else {
                println!("{}└── {}", prefix, key);
                print_level(subtree, [&is_last, [true].as_ref()].concat());
            }
        }
    }

    print_level(&tree, vec![]);
}

/// Defines a unit type with an associated symbol for metric formatting.
pub trait Unit: Sized {
    /// The symbol to be displayed after the numeric value (e.g., "B" for bytes).
    const SYMBOL: &'static str;
}

/// A wrapper for numeric values that formats them with appropriate metric prefixes.
///
/// Automatically scales values and adds appropriate SI prefixes (k, M, G, T) when
/// displaying the value.
pub struct Metric<U: Unit> {
    /// The numeric value to be formatted
    value: f64,
    /// Phantom data to carry the unit type
    _phantom: std::marker::PhantomData<U>,
}

impl<U: Unit> Display for Metric<U> {
    /// Formats the value with appropriate metric prefix and unit symbol.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (divider, multiplier) = match self.value {
            v if v < 1e-9 => (1e-12, "p"),
            v if v < 1e-6 => (1e-9, "n"),
            v if v < 1e-3 => (1e-6, "μ"),
            v if v < 1.0 => (1e-3, "m"),
            v if v < 1e3 => (1.0, ""),
            v if v < 1e6 => (1e3, "k"),
            v if v < 1e9 => (1e6, "M"),
            v if v < 1e12 => (1e9, "G"),
            _ => (1e12, "T"),
        };

        write!(f, "{} {}{}", self.value / divider, multiplier, U::SYMBOL)
    }
}
