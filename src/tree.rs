use crate::ui::{accent, muted};
use std::collections::BTreeMap;

/// A rendered path tree plus the order its files were printed in.
///
/// `order[i]` is the index into the input paths of the file printed i-th.
/// When the tree is numbered, that position is the id shown in the gutter, so
/// ids always read top to bottom.
pub struct Rendered {
    pub text: String,
    pub order: Vec<usize>,
}

pub fn render_tree(paths: &[String], numbered: bool) -> Rendered {
    let mut root = Node::default();
    for (index, path) in paths.iter().enumerate() {
        root.insert(path, index);
    }

    let gutter = if numbered {
        paths.len().saturating_sub(1).to_string().len()
    } else {
        0
    };

    let mut out = Out {
        lines: Vec::new(),
        order: Vec::new(),
        gutter,
        numbered,
    };
    root.render_children(&mut out, None);

    Rendered {
        text: out.lines.join("\n"),
        order: out.order,
    }
}

#[derive(Default)]
struct Node {
    children: BTreeMap<String, Node>,
    files: Vec<usize>,
}

struct Out {
    lines: Vec<String>,
    order: Vec<usize>,
    gutter: usize,
    numbered: bool,
}

impl Out {
    fn push(&mut self, prefix: &str, label: String, file: Option<usize>) {
        let gutter = match (self.numbered, file) {
            (false, _) => String::new(),
            (true, Some(_)) => format!("{:>w$}  ", self.order.len(), w = self.gutter),
            (true, None) => " ".repeat(self.gutter + 2),
        };
        if let Some(index) = file {
            self.order.push(index);
        }
        self.lines.push(format!("{gutter}{prefix}{label}"));
    }
}

impl Node {
    fn insert(&mut self, path: &str, index: usize) {
        let mut current = self;
        for segment in path.split('/').filter(|s| !s.is_empty()) {
            current = current.children.entry(segment.to_string()).or_default();
        }
        current.files.push(index);
    }

    /// `prefix` is `None` at the root, where entries print flush left without
    /// branch glyphs.
    fn render_children(&self, out: &mut Out, prefix: Option<&str>) {
        let rows = self
            .children
            .iter()
            .flat_map(|(name, child)| {
                let files = child.files.iter().map(move |&i| Row::File(name, i));
                let dir = (!child.children.is_empty()).then_some(Row::Dir(name, child));
                files.chain(dir)
            })
            .collect::<Vec<_>>();

        let count = rows.len();
        for (i, row) in rows.into_iter().enumerate() {
            let last = i + 1 == count;
            let (head, tail) = match prefix {
                None => (String::new(), String::new()),
                Some(p) => {
                    let branch = if last { "└── " } else { "├── " };
                    let cont = if last { "    " } else { "│   " };
                    (
                        format!("{p}{}", muted(branch)),
                        format!("{p}{}", muted(cont)),
                    )
                }
            };

            match row {
                Row::File(name, index) => out.push(&head, file_label(name), Some(index)),
                Row::Dir(name, child) => {
                    let (joined, inner) = child.collapse(name);
                    out.push(&head, dir_label(&joined, prefix.is_none()), None);
                    inner.render_children(out, Some(&tail));
                }
            }
        }
    }

    /// Fold chains of single-child directories into one `a/b/c/` row.
    fn collapse<'a>(&'a self, name: &str) -> (String, &'a Node) {
        let mut parts = vec![name];
        let mut current = self;
        while current.files.is_empty() && current.children.len() == 1 {
            let (child_name, child) = current.children.iter().next().expect("one child");
            if child.children.is_empty() {
                break;
            }
            parts.push(child_name);
            current = child;
        }
        (format!("{}/", parts.join("/")), current)
    }
}

enum Row<'a> {
    File(&'a str, usize),
    Dir(&'a str, &'a Node),
}

fn dir_label(name: &str, top_level: bool) -> String {
    if top_level && let Some((source, rest)) = name.split_once(':') {
        return format!("{}{}", muted(&format!("{source}:")), rest);
    }
    name.to_string()
}

fn file_label(name: &str) -> String {
    for ext in [".json.xz", ".json.gz", ".json"] {
        if let Some(base) = name.strip_suffix(ext) {
            return format!("{}{}", accent(base), muted(ext));
        }
    }
    accent(name).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(paths: &[&str], numbered: bool) -> Rendered {
        colored::control::set_override(false);
        let paths = paths.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        render_tree(&paths, numbered)
    }

    #[test]
    fn collapses_single_child_directories() {
        let r = plain(
            &[
                "abyss:Debian/amd64/6.1.0/debian.json.xz",
                "abyss:Debian/amd64/6.1.0/debian-debug.json.xz",
            ],
            true,
        );
        assert_eq!(
            r.text,
            "   abyss:Debian/amd64/6.1.0/\n\
             0  ├── debian-debug.json.xz\n\
             1  └── debian.json.xz"
        );
        assert_eq!(r.order, vec![1, 0]);
    }

    #[test]
    fn ids_follow_display_order_not_input_order() {
        let r = plain(
            &[
                "abyss:Ubuntu/amd64/b.json.xz",
                "leludo:profiles/a.json.xz",
                "abyss:Ubuntu/amd64/a.json.xz",
            ],
            true,
        );
        assert_eq!(r.order, vec![2, 0, 1]);
        let ids = r
            .text
            .lines()
            .filter_map(|l| l.split_whitespace().next()?.parse::<usize>().ok())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![0, 1, 2]);
    }

    #[test]
    fn gutter_right_aligns_multi_digit_ids() {
        let paths = (0..11)
            .map(|i| format!("p0d:linux/{i:02}.json"))
            .collect::<Vec<_>>();
        let refs = paths.iter().map(String::as_str).collect::<Vec<_>>();
        let r = plain(&refs, true);
        let lines = r.text.lines().collect::<Vec<_>>();
        assert_eq!(lines[0], "    p0d:linux/");
        assert!(lines[1].starts_with(" 0  ├── "));
        assert!(lines[11].starts_with("10  └── "));
    }

    #[test]
    fn unnumbered_tree_has_no_gutter() {
        let r = plain(&["linux/a.json.xz", "mac/b.json.xz"], false);
        assert_eq!(r.text, "linux/\n└── a.json.xz\nmac/\n└── b.json.xz");
    }

    #[test]
    fn duplicate_paths_each_get_a_row() {
        let r = plain(&["abyss:x/a.json", "abyss:x/a.json"], true);
        assert_eq!(r.order.len(), 2);
    }
}
