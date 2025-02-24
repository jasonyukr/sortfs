use clap::{App, Arg};
use std::cmp::{min};
use std::io;
use std::path::PathBuf;
use std::time::SystemTime;
use rayon::prelude::*;
use ignore::{WalkBuilder, DirEntry, overrides::OverrideBuilder};
use std::sync::{Arc, Mutex};
use std::fs::metadata;


fn is_dir(entry: &DirEntry) -> bool {
    entry
        .file_type()
        .as_ref()
        .map(|f| f.is_dir())
        .unwrap_or(false)
}

fn build_entries(dirs_only: bool, current_dir: &PathBuf) -> Vec<DirEntry> {
    // Use a maximum of 4 threads. Never use more than half of the available CPU cores.
    let num_threads = min(4, num_cpus::get() / 2);

    let results = Arc::new(Mutex::new(vec![]));

    let mut builder = WalkBuilder::new(&current_dir);

    // make sure that ".git/" contents are ignored
    let mut overrides = OverrideBuilder::new(&current_dir);
    overrides.add("!**/.git/").unwrap();
    builder.overrides(overrides.build().unwrap());

    let walker;
    if dirs_only {
        walker = builder
            .hidden(false)
            .follow_links(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .filter_entry(is_dir)
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

    walker.run(|| {
        let results = Arc::clone(&results);
        Box::new(move |entry| {
            match entry {
                Ok(entry) => {
                    let mut results = results.lock().unwrap();
                    results.push(entry);
                }
                Err(err) => eprintln!("Error: {}", err),
            }
            ignore::WalkState::Continue
        })
    });

    // sort by modified date
    let mut results = results.lock().unwrap();
    results.par_sort_by(|a, b| {
        let a_metadata = metadata(a.path()).unwrap();
        let b_metadata = metadata(b.path()).unwrap();
        let a_modified = a_metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let b_modified = b_metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        b_modified.cmp(&a_modified)
    });

    results.to_vec()
}

fn main() -> io::Result<()> {
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
                .takes_value(false)
        )
        .get_matches();

    let dirs_only = matches.is_present("dirs-only");

    let dir = PathBuf::from(matches.value_of("DIRECTORY").unwrap_or("."));
    let entries = build_entries(dirs_only, &dir);
    let mut leading_path = dir.to_str().unwrap();
    leading_path = leading_path.trim_end_matches('/');

    for e in &entries {
        let path = format!("{}", e.path().display());
        if path.len() > leading_path.len() {
            if e.path().is_dir() {
                println!("{}/", &path[leading_path.len() + 1..]);
            } else {
                println!("{}", &path[leading_path.len() + 1..]);
            }
        }
    }

    Ok(())
}
