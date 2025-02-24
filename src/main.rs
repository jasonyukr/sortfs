use clap::{App, Arg};
use jwalk::{DirEntry, WalkDir};
use std::cmp::{min, Reverse};
use std::error::Error;
use std::fs::Metadata;
use std::io;
use std::path::PathBuf;
use std::time::SystemTime;
use rayon::prelude::*;

#[derive(PartialEq, Eq)]
enum SortBy {
    Modified,
    Created,
}

fn is_dir(entry: &DirEntry) -> bool {
    entry
        .file_type
        .as_ref()
        .map(|f| f.is_dir())
        .unwrap_or(false)
}

fn metadata_result<F>(e: &DirEntry, process: F) -> Result<u64, Box<dyn Error>>
where
    F: Fn(&Metadata) -> io::Result<SystemTime>,
{
    let metadata: Option<&Metadata> = e.metadata.as_ref().unwrap().as_ref().ok();
    if let Some(metadata) = metadata {
        Ok(process(metadata)?
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs())
    } else {
        Err(Box::new(io::Error::new(
            io::ErrorKind::Other,
            "Couldn't get metadata",
        )))
    }
}

fn mtime(e: &DirEntry, default: u64) -> u64 {
    metadata_result(e, |metadata| metadata.modified()).unwrap_or(default)
}

fn ctime(e: &DirEntry, default: u64) -> u64 {
    metadata_result(e, |metadata| metadata.created()).unwrap_or(default)
}

fn build_entries(dirs_only: bool, current_dir: &PathBuf, sort_by: SortBy) -> Vec<DirEntry> {
    // Use a maximum of 4 threads. Never use more than half of the available CPU cores.
    let num_threads = min(4, num_cpus::get() / 2);
    let walker = WalkDir::new(&current_dir)
        .skip_hidden(false)
        .preload_metadata(true)
        .num_threads(num_threads);

    let mut x: Vec<DirEntry>;
    if dirs_only {
        x = walker
            .into_iter()
            // skip items that we can't access
            .filter_map(Result::ok)
            // takes directory only
            .filter(is_dir)
            .collect();
    } else {
        x = walker
            .into_iter()
            // skip items that we can't access
            .filter_map(Result::ok)
            .collect();
    }

    // Filter out entries under the .git directory
    x.retain(|entry| {
        !entry.path().components().any(|component| component.as_os_str() == ".git")
    });

    // Sort entries by stat date using rayon
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    x.par_sort_by_cached_key(|e| match sort_by {
        SortBy::Modified => Reverse(mtime(e, now)),
        SortBy::Created => Reverse(ctime(e, now)),
    });
    x.into_iter().collect()
}

fn main() -> io::Result<()> {
    let matches = App::new("sortfs")
        .version("1.0")
        .arg(
            Arg::with_name("sort-by")
                .short("s")
                .long("sort-by")
                .value_name("SORT_BY")
                .help("Sort by an attribute (defaults to modified)")
                .takes_value(true)
                .possible_values(&["modified", "created"]),
        )
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
    let sort_by = match matches.value_of("sort-by").unwrap_or("modified") {
        "created" => SortBy::Created,
        "modified" => SortBy::Modified,
        _ => SortBy::Modified,
    };

    let dirs_only = matches.is_present("dirs-only");

    let dir = PathBuf::from(matches.value_of("DIRECTORY").unwrap_or("."));
    let entries = build_entries(dirs_only, &dir, sort_by);
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
