use core::fmt;
use std::{collections::BTreeMap, fmt::Display};

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

pub trait Unit: Sized {
    const SYMBOL: &'static str;
    fn value(value: f64) -> Metric<Self> {
        Metric {
            value,
            _phantom: std::marker::PhantomData,
        }
    }
}

pub struct Metric<U: Unit> {
    value: f64,
    _phantom: std::marker::PhantomData<U>,
}

impl<U: Unit> Display for Metric<U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (divider, multiplier) = if self.value < 1e3 {
            (1.0, "")
        } else if self.value < 1e6 {
            (1e3, "k")
        } else if self.value < 1e9 {
            (1e6, "M")
        } else if self.value < 1e12 {
            (1e9, "G")
        } else {
            (1e12, "T")
        };

        write!(f, "{} {}{}", self.value / divider, multiplier, U::SYMBOL)
    }
}
