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

fn print_path(handle: &mut dyn Write, path: &str, is_dir: bool) -> io::Result<()> {
    write!(handle, "{}", path)?;
    if is_dir && !path.eq("/") {
        write!(handle, "/")?;
    }
    writeln!(handle)?;
    Ok(())
}

fn print_lscolor_path(handle: &mut dyn Write, ls_colors: &LsColors, path: &str, is_dir: bool) -> io::Result<()> {
    for (component, style) in ls_colors.style_for_path_components(Path::new(path)) {
        #[cfg(any(feature = "nu-ansi-term", feature = "gnu_legacy"))]
        {
            let ansi_style = style.map(Style::to_nu_ansi_term_style).unwrap_or_default();
            write!(handle, "{}", ansi_style.paint(component.to_string_lossy()))?;
        }
    }
    if is_dir && !path.eq("/") {
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

fn build_entries(dirs_only: bool, max_depth: Option<usize>, current_dir: &PathBuf) -> Vec<(DirEntry, SystemTime)> {
    // Use reasonable number of threads
    let num_threads = min(8, num_cpus::get() / 2);

    // Builder for current_dir
    let mut builder = WalkBuilder::new(&current_dir);

    // Ignore ".git/" sub-path
    let mut overrides = OverrideBuilder::new(&current_dir);
    overrides.add("!**/.git/*").unwrap();
    builder.overrides(overrides.build().unwrap());

    // Create walker from builder
    let walker;
    if dirs_only {
        walker = builder
            .standard_filters(true)
            .add_custom_ignore_filename(".fdignore")
            .hidden(false)
            .follow_links(true)
            .filter_entry(is_dir) // directory only
            .max_depth(max_depth)
            .threads(num_threads)
            .build_parallel();
    } else {
        walker = builder
            .standard_filters(true)
            .add_custom_ignore_filename(".fdignore")
            .hidden(false)
            .follow_links(true)
            .max_depth(max_depth)
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
        .arg(
            Arg::with_name("color")
                .short("c")
                .long("color")
                .help("Use ls-colors")
        )
        .arg(
            Arg::with_name("prefix-target")
                .short("p")
                .long("prefix-target")
                .help("Put the target-dir as prefix")
        )
        .arg(
            Arg::with_name("max-depth")
                .short("m")
                .long("max-depth")
                .takes_value(true)
                .help("max depth for directory walk through")
        )
        .get_matches();

    let dirs_only = matches.is_present("dirs-only");
    let full_path = matches.is_present("full-path");
    let color = matches.is_present("color");
    let mut prefix_target = matches.is_present("prefix-target");
    if full_path {
        prefix_target = false;
    }

    let mut target_dir = matches.value_of("DIRECTORY").unwrap_or(".");
    target_dir = target_dir.trim_end_matches('/');

    let max_depth = matches.value_of("max-depth").unwrap_or("");
    let max_depth: Option<usize> = match max_depth.parse::<usize>() {
        Ok(n) => Some(n),
        Err(_) => None
    };

    let dir;
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
    let entries = build_entries(dirs_only, max_depth, &dir);
    let mut leading_path = dir.to_str().unwrap();
    leading_path = leading_path.trim_end_matches('/');

    for e in &entries {
        let path = e.0.path();
        let path_disp;
        if prefix_target {
            path_disp = format!("{}/{}", target_dir, path.display());
        } else {
            path_disp = format!("{}", path.display());
        }
        let res;
        if full_path {
            if color {
                res = print_lscolor_path(&mut stdout, &ls_colors, path_disp.as_ref(), path.is_dir());
            } else {
                res = print_path(&mut stdout, path_disp.as_ref(), path.is_dir());
            }
        } else {
            if path_disp.len() > leading_path.len() {
                if color {
                    res = print_lscolor_path(&mut stdout, &ls_colors, path_disp[leading_path.len() + 1..].as_ref(), path.is_dir());
                } else {
                    res = print_path(&mut stdout, path_disp[leading_path.len() + 1..].as_ref(), path.is_dir());
                }
            } else {
                res = Ok(());
            }
        }
        if let Err(_) = res {
            process::exit(1);
        }
    }

    Ok(())
}
