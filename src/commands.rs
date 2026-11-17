use crate::cache::{read_last_search_results, write_last_search_results};
use crate::github::{Download, download_symbol};
use crate::search::search_entries;
use crate::sources::{SOURCES, SymbolEntry, fetch_all_sources};
use crate::symbols::{dest_dir_for, find_symbols_dir, list_local_symbols};
use crate::tree::render_tree;
use crate::ui::{self, Ledger, muted, plural, strong, success, user_error};
use anyhow::Result;

fn fetch_indexes() -> Result<Vec<SymbolEntry>> {
    let pb = ui::spinner(SOURCES.len() as u64, "fetching indexes");
    let result = fetch_all_sources(|| pb.inc(1));
    pb.finish_and_clear();
    result
}

/// Render entries as a numbered tree and cache them in display order so the
/// printed ids resolve with `vol3sm install <id>`.
/// Returns the tree text and the input index of the entry printed as id 0.
fn numbered_tree(query: &str, entries: &[&SymbolEntry]) -> Result<(String, usize)> {
    let paths = entries.iter().map(|e| e.display_path()).collect::<Vec<_>>();
    let rendered = render_tree(&paths, true);
    let ordered = rendered
        .order
        .iter()
        .map(|&i| entries[i])
        .collect::<Vec<_>>();
    write_last_search_results(query, &ordered)?;
    Ok((rendered.text, rendered.order.first().copied().unwrap_or(0)))
}

pub fn cmd_search(query: &str, limit: usize) -> Result<()> {
    let entries = fetch_indexes()?;
    let results = search_entries(&entries, query);

    if results.is_empty() {
        println!("No matches for {}", strong(query));
        return Ok(());
    }

    let total = results.len();
    let shown = &results[..total.min(limit.max(1))];
    let (tree, _) = numbered_tree(query, shown)?;

    let count = if shown.len() < total {
        format!(
            "{} of {}",
            strong(&shown.len().to_string()),
            strong(&plural(total, "match", "matches"))
        )
    } else {
        strong(&plural(total, "match", "matches")).to_string()
    };
    println!("{count} for {query}\n");
    println!("{tree}\n");

    let mut next = Ledger::new().row("install", "vol3sm install <id>");
    if shown.len() < total {
        next = next.row("more", format!("--limit {total}"));
    }
    println!("{next}");
    Ok(())
}

pub fn cmd_install(path: Option<String>, banner: Option<String>) -> Result<()> {
    let entry = match (path, banner) {
        (Some(p), _) => resolve_path(&p)?,
        (None, Some(b)) => resolve_banner(&b)?,
        (None, None) => unreachable!("clap requires a path or --banner"),
    };

    let symbols_dir = find_symbols_dir()?;
    let dest = dest_dir_for(&symbols_dir, &entry.path);
    std::fs::create_dir_all(&dest)?;

    println!("{}", symbol_label(&entry));
    match download_symbol(&entry.download_url, &dest, &entry.path)? {
        Download::Installed(file) => println!("{}  {}", success("installed"), file.display()),
        Download::AlreadyPresent(file) => {
            println!("{}  {}", muted("already installed"), file.display())
        }
    }
    Ok(())
}

fn symbol_label(entry: &SymbolEntry) -> String {
    format!("{}{}", muted(&format!("{}:", entry.source_id)), entry.path)
}

fn resolve_banner(banner: &str) -> Result<SymbolEntry> {
    let banner = banner.trim();
    let matches = fetch_indexes()?
        .into_iter()
        .filter(|entry| !entry.banner.is_empty() && entry.banner.trim() == banner)
        .collect::<Vec<_>>();
    pick_first(matches, banner, || {
        user_error(
            "No symbol matches this banner",
            Some("vol3sm search \"<distro> <kernel version>\""),
        )
    })
}

fn resolve_path(path: &str) -> Result<SymbolEntry> {
    if let Ok(id) = path.parse::<usize>() {
        return resolve_search_id(id);
    }

    let matches = fetch_indexes()?
        .into_iter()
        .filter(|entry| entry.display_path() == path || (!path.contains(':') && entry.path == path))
        .collect::<Vec<_>>();
    pick_first(matches, path, || {
        user_error(
            format!("No symbol at {path}"),
            Some("vol3sm search <query>"),
        )
    })
}

/// Several sources can carry the same symbol. Show them all as a numbered
/// tree, install the first, and leave the rest one `install <id>` away.
fn pick_first(
    matches: Vec<SymbolEntry>,
    query: &str,
    none: impl FnOnce() -> anyhow::Error,
) -> Result<SymbolEntry> {
    match matches.len() {
        0 => Err(none()),
        1 => Ok(matches.into_iter().next().expect("one match")),
        n => {
            let refs = matches.iter().collect::<Vec<_>>();
            let (tree, first) = numbered_tree(query, &refs)?;
            println!("{}, installing 0\n", strong(&plural(n, "match", "matches")));
            println!("{tree}\n");
            Ok(matches[first].clone())
        }
    }
}

fn resolve_search_id(id: usize) -> Result<SymbolEntry> {
    let cache = read_last_search_results()
        .map_err(|_| user_error("No previous search", Some("vol3sm search <query>")))?;
    let last = cache.entries.len().saturating_sub(1);
    cache.entries.get(id).cloned().ok_or_else(|| {
        user_error(
            format!("No id {id} in the last search, ids run 0 to {last}"),
            Some(&rerun_hint(&cache.query)),
        )
    })
}

/// Repeat the last query when it is short enough to retype, such as a search
/// term. A cached banner is not.
fn rerun_hint(query: &str) -> String {
    if query.chars().count() <= 40 {
        format!("vol3sm search \"{query}\"")
    } else {
        "vol3sm search <query>".to_string()
    }
}

pub fn cmd_local() -> Result<()> {
    let symbols_dir = find_symbols_dir()?;
    let files = list_local_symbols(&symbols_dir)?;

    if files.is_empty() {
        println!("No symbols in {}\n", symbols_dir.display());
        println!("{}", Ledger::new().row("find", "vol3sm search <query>"));
        return Ok(());
    }

    let paths = files
        .iter()
        .map(|f| {
            f.strip_prefix(&symbols_dir)
                .unwrap_or(f)
                .display()
                .to_string()
        })
        .collect::<Vec<_>>();

    println!(
        "{} in {}\n",
        strong(&plural(files.len(), "symbol", "symbols")),
        symbols_dir.display()
    );
    println!("{}", render_tree(&paths, false).text);
    Ok(())
}

pub fn cmd_symbols_dir() -> Result<()> {
    println!("{}", find_symbols_dir()?.display());
    Ok(())
}
