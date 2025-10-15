use clap::{App, Arg};
use ignore::{overrides::OverrideBuilder, DirEntry, WalkBuilder};
use lscolors::{LsColors, Style};
use rayon::prelude::*;
use std::fs;
use std::fs::metadata;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::mpsc;
use std::time::SystemTime;

#[cfg(all(not(feature = "nu-ansi-term")))]
compile_error!("feature must be enabled: nu-ansi-term");

#[inline]
fn is_dir(entry: &DirEntry) -> bool {
    entry
        .file_type()
        .as_ref()
        .map(|f| f.is_dir())
        .unwrap_or(false)
}

#[inline]
fn print_path<W: Write>(handle: &mut W, path: &Path, is_dir: bool) -> io::Result<()> {
    write!(handle, "{}", path.display())?;
    if is_dir && path != Path::new("/") {
        write!(handle, "/")?;
    }
    writeln!(handle)?;
    Ok(())
}

#[inline]
fn print_lscolor_path<W: Write>(
    handle: &mut W,
    ls_colors: &LsColors,
    path: &Path,
    is_dir: bool,
) -> io::Result<()> {
    for (component, style) in ls_colors.style_for_path_components(path) {
        #[cfg(any(feature = "nu-ansi-term", feature = "gnu_legacy"))]
        {
            let ansi_style = style.map(Style::to_nu_ansi_term_style).unwrap_or_default();
            write!(handle, "{}", ansi_style.paint(component.to_string_lossy()))?;
        }
    }
    if is_dir && path != Path::new("/") {
        write!(handle, "/")?;
    }
    writeln!(handle)?;
    Ok(())
}

fn build_entries(
    dirs_only: bool,
    max_depth: Option<usize>,
    current_dir: &Path,
    leftover: Option<String>,
) -> Vec<(PathBuf, bool, SystemTime)> {
    // Use all logical cores for traversal
    let num_threads = num_cpus::get();

    // Builder for current_dir
    let mut builder = WalkBuilder::new(current_dir);

    // Ignore ".git/" sub-path
    let mut overrides = OverrideBuilder::new(current_dir);
    overrides.add("!**/.git/*").unwrap();
    builder.overrides(overrides.build().unwrap());

    // Configure walker
    let mut builder = builder
        .standard_filters(true)
        .add_custom_ignore_filename(".fdignore")
        .hidden(false)
        .follow_links(true)
        .max_depth(max_depth)
        .threads(num_threads);

    // Apply filtering as early as possible to reduce I/O.
    // When a leftover string is provided, restrict traversal by precomputing
    // the matching top-level children (files/dirs) once, then only traverse
    // entries that are under those paths. This minimizes per-entry work.
    let base = current_dir.to_path_buf();
    if let Some(filter) = leftover {
        // Pre-scan direct children of base to compute allowed roots.
        let allowed_roots: Vec<PathBuf> = match fs::read_dir(&base) {
            Ok(read_dir) => read_dir
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let name = e.file_name();
                    match name.to_str() {
                        Some(s) if s.starts_with(&filter) => Some(e.path()),
                        _ => None,
                    }
                })
                .collect(),
            Err(_) => Vec::new(),
        };

        if dirs_only {
            let allowed = allowed_roots.clone();
            builder = builder.filter_entry(move |entry| {
                let p = entry.path();
                if p == base {
                    return true;
                }
                if !is_dir(entry) {
                    // Don't include files when dirs-only is set.
                    return false;
                }
                // Descend only into directories under allowed roots.
                allowed.iter().any(|root| p.starts_with(root))
            });
        } else {
            let allowed = allowed_roots.clone();
            builder = builder.filter_entry(move |entry| {
                let p = entry.path();
                if p == base {
                    return true;
                }
                // Keep entries that are either the matching top-level files, or under matching directories.
                allowed.iter().any(|root| p.starts_with(root))
            });
        }
    } else if dirs_only {
        // Only show directories
        builder = builder.filter_entry(move |entry| is_dir(entry));
    }

    let walker = builder.build_parallel();

    // Collect results without a global mutex; use channel to reduce contention
    let (tx, rx) = mpsc::channel::<(PathBuf, bool, SystemTime)>();

    walker.run(|| {
        let tx = tx.clone();
        Box::new(move |entry| {
            if let Ok(entry) = entry {
                let path = entry.path().to_path_buf();

                // Determine dir flag without extra syscalls where possible
                let is_dir_flag = entry
                    .file_type()
                    .as_ref()
                    .map(|ft| ft.is_dir())
                    .unwrap_or_else(|| path.is_dir());

                // Fetch mtime; default to UNIX_EPOCH on error
                let modified = metadata(&path)
                    .and_then(|meta| meta.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);

                // Ignore send errors if receiver is gone
                if tx.send((path, is_dir_flag, modified)).is_err() {
                    return ignore::WalkState::Quit;
                }
            }
            ignore::WalkState::Continue
        })
    });
    drop(tx);

    // Drain channel
    let mut results: Vec<(PathBuf, bool, SystemTime)> = rx.into_iter().collect();

    // Remove the walk target itself if present
    results.retain(|(path, _, _)| path != current_dir);

    // Sort by mtime DESC; tie-break by path ASC for deterministic ordering across runs
    results.par_sort_unstable_by(|(pa, _, ma), (pb, _, mb)| {
        mb.cmp(ma).then_with(|| pa.cmp(pb))
    });

    results
}

fn normalize_path(path: &str) -> io::Result<PathBuf> {
    let path = Path::new(path);
    fs::canonicalize(path)
}

fn main() -> io::Result<()> {
    let ls_colors = LsColors::from_env().unwrap_or_default();

    // Buffered stdout to minimize syscalls during printing
    let stdout = io::stdout();
    let mut out = BufWriter::with_capacity(64 * 1024, stdout.lock());

    let matches = App::new("sortfs")
        .version("1.0")
        .arg(
            Arg::with_name("PREFIX")
                .help("Target directory to walk through (defaults to current directory)")
                .index(1),
        )
        .arg(
            Arg::with_name("LEFTOVER")
                .help("Leftover used to filter the result (defaults to \"\")")
                .index(2),
        )
        .arg(
            Arg::with_name("dirs-only")
                .short("d")
                .long("dirs-only")
                .help("Show directories only"),
        )
        .arg(
            Arg::with_name("full-path")
                .short("f")
                .long("full-path")
                .help("Show fullpath"),
        )
        .arg(
            Arg::with_name("color")
                .short("c")
                .long("color")
                .help("Use ls-colors"),
        )
        .arg(
            Arg::with_name("max-depth")
                .short("m")
                .long("max-depth")
                .takes_value(true)
                .help("max depth for directory walk through"),
        )
        .get_matches();

    let dirs_only = matches.is_present("dirs-only");
    let full_path = matches.is_present("full-path");
    let color = matches.is_present("color");

    let mut target_dir = matches.value_of("PREFIX").unwrap_or(".");
    target_dir = target_dir.trim_end_matches('/');

    let leftover_val = matches.value_of("LEFTOVER").unwrap_or("");

    let max_depth = matches.value_of("max-depth").unwrap_or("");
    let max_depth: Option<usize> = match max_depth.parse::<usize>() {
        Ok(n) => Some(n),
        Err(_) => None,
    };

    let prefix_dir: PathBuf;
    let leftover: Option<String>;
    if full_path {
        match normalize_path(target_dir) {
            Ok(normalized) => {
                prefix_dir = normalized.clone();
                if !leftover_val.is_empty() {
                    leftover = Some(leftover_val.to_string());
                } else {
                    leftover = None;
                }
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
    } else {
        prefix_dir = PathBuf::from(target_dir);
        if !leftover_val.is_empty() {
            leftover = Some(leftover_val.to_string());
        } else {
            leftover = None;
        }
    }

    let entries = build_entries(dirs_only, max_depth, &prefix_dir, leftover);

    for (path, is_dir, _modified) in &entries {
        let res = if color {
            print_lscolor_path(&mut out, &ls_colors, path, *is_dir)
        } else {
            print_path(&mut out, path, *is_dir)
        };
        if res.is_err() {
            process::exit(1);
        }
    }

    // Ensure all output is flushed
    out.flush()?;

    Ok(())
}
