//! Small utilities used across the CLI.

use std::collections::BTreeMap;

/// Red bold asterisk, used as a visual marker in CLI output.
pub const MARKER: &str = "\u{001b}[1m\u{001b}[31m*\u{001b}[0m";

/// Prints a list of paths as a tree, like the `tree(1)` command.
///
/// Splits each path on `delimiter` and groups by shared prefix, drawing the
/// branches with `|--` style indicators between segments.
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
