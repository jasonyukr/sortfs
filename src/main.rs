use clap::{App, Arg};
use std::cmp::{min};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::SystemTime;
use std::process;
use std::sync::{Arc, Mutex};
use std::fs::metadata;
use rayon::prelude::*;
use ignore::{WalkBuilder, DirEntry, overrides::OverrideBuilder};
use std::path::Path;
use std::fs;

use lscolors::{LsColors, Style};

#[cfg(all(
    not(feature = "nu-ansi-term"),
))]
compile_error!(
    "feature must be enabled: nu-ansi-term"
);

fn print_path(handle: &mut dyn Write, ls_colors: &LsColors, path: &str, trailing_slash: bool) -> io::Result<()> {
    for (component, style) in ls_colors.style_for_path_components(Path::new(path)) {
        #[cfg(any(feature = "nu-ansi-term", feature = "gnu_legacy"))]
        {
            let ansi_style = style.map(Style::to_nu_ansi_term_style).unwrap_or_default();
            write!(handle, "{}", ansi_style.paint(component.to_string_lossy()))?;
        }
    }
    if trailing_slash {
        write!(handle, "/")?;
    }
    writeln!(handle)?;

    Ok(())
}

fn is_dir(entry: &DirEntry) -> bool {
    entry
        .file_type()
        .as_ref()
        .map(|f| f.is_dir())
        .unwrap_or(false)
}

fn build_entries(dirs_only: bool, current_dir: &PathBuf) -> Vec<(DirEntry, SystemTime)> {
    // Use a maximum of 4 threads. Never use more than half of the available CPU cores.
    let num_threads = min(4, num_cpus::get() / 2);

    let mut builder = WalkBuilder::new(&current_dir);

    // Make sure that ".git/" contents are ignored
    let mut overrides = OverrideBuilder::new(&current_dir);
    overrides.add("!**/.git/").unwrap();
    builder.overrides(overrides.build().unwrap());

    // Create walker
    let walker;
    if dirs_only {
        walker = builder
            .hidden(false)
            .follow_links(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .filter_entry(is_dir) // directory only
            .threads(num_threads)
            .build_parallel();
    } else {
        walker = builder
            .hidden(false)
            .follow_links(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .threads(num_threads)
            .build_parallel();
    }

    // Run the walker to collect (entry, modified) vector
    let results = Arc::new(Mutex::new(Vec::new()));
    walker.run(|| {
        let results = Arc::clone(&results);
        Box::new(move |entry| {
            match entry {
                Ok(entry) => {
                    let modified = metadata(entry.path())
                        .and_then(|meta| meta.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH); // default to UNIX_EPOCH if error
                    let mut results = results.lock().unwrap();
                    results.push((entry, modified));
                }
                Err(_err) => (),
            }
            ignore::WalkState::Continue
        })
    });

    // Sort the results by the "modified"
    let mut results = results.lock().unwrap();
    results.par_sort_by(|(_a, a_modified), (_b, b_modified)| {
        b_modified.cmp(&a_modified)
    });

    results.to_vec()
}

fn normalize_path(path: &str) -> std::io::Result<String> {
    let path = Path::new(path);
    let canonical_path = fs::canonicalize(path)?;
    Ok(canonical_path.to_string_lossy().into_owned())
}

fn main() -> io::Result<()> {
    let ls_colors = LsColors::from_env().unwrap_or_default();

    let mut stdout = io::stdout();
    let matches = App::new("sortfs")
        .version("1.0")
        .arg(
            Arg::with_name("DIRECTORY")
                .help("Directory to walk through (defaults to current directory)")
                .index(1),
        )
        .arg(
            Arg::with_name("dirs-only")
                .short("d")
                .long("dirs-only")
                .help("Show directories only")
        )
        .arg(
            Arg::with_name("full-path")
                .short("f")
                .long("full-path")
                .help("Show fullpath")
        )
        .get_matches();

    let dirs_only = matches.is_present("dirs-only");
    let full_path = matches.is_present("full-path");

    let dir;
    let target_dir = matches.value_of("DIRECTORY").unwrap_or(".");
    if full_path {
        match normalize_path(target_dir) {
            Ok(normalized) => dir = PathBuf::from(normalized),
            Err(e) => {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
    } else {
        dir = PathBuf::from(target_dir);
    }
    let entries = build_entries(dirs_only, &dir);
    let mut leading_path = dir.to_str().unwrap();
    leading_path = leading_path.trim_end_matches('/');

    for e in &entries {
        let path = format!("{}", e.0.path().display());
        let res;
        if full_path {
            res = print_path(&mut stdout, &ls_colors, path.as_ref(), e.0.path().is_dir());
        } else {
            if path.len() > leading_path.len() {
                res = print_path(&mut stdout, &ls_colors, path[leading_path.len() + 1..].as_ref(), e.0.path().is_dir());
            } else {
                res = Ok(());
            }
        }
        match res {
            Ok(_) => (),
            Err(_e) => { process::exit(1) },
        }
    }

    Ok(())
}
