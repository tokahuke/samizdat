//! General utilities for the CLI application.
//!
//! This module provides utility functions and types for common operations across the CLI.

use std::collections::BTreeMap;

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

