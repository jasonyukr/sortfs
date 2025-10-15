use assert_cmd::prelude::*;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn write_file(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut f = File::create(path).unwrap();
    f.write_all(contents).unwrap();
}

fn run_and_stdout(args: &[&str], current_dir: &Path) -> String {
    let mut cmd = Command::cargo_bin("sortfs").unwrap();
    let output = cmd
        .current_dir(current_dir)
        // Ensure LS_COLORS won't inject unpredictable ANSI sequences
        .env_remove("LS_COLORS")
        .args(args)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "command failed: status={:?}, stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn lines(stdout: &str) -> Vec<String> {
    stdout.lines().map(|s| s.to_string()).collect()
}

#[test]
fn cli_basic_count_with_full_path_depth1() {
    let td = tempdir().unwrap();
    let base = td.path();
    let d1 = base.join("dir1");
    let f1 = base.join("file1.txt");
    fs::create_dir_all(&d1).unwrap();
    write_file(&f1, b"hello");

    let out = run_and_stdout(&["-f", "-m", "1", "."], base);
    let ls = lines(&out);
    // Only direct children of base are listed (depth = 1)
    // Should be exactly 2 entries: dir1/ and file1.txt (order not guaranteed)
    assert_eq!(ls.len(), 2, "stdout:\n{}", out);
}

#[test]
fn cli_dirs_only_excludes_files() {
    let td = tempdir().unwrap();
    let base = td.path();
    let d1 = base.join("dir1");
    let f1 = base.join("file1.txt");
    fs::create_dir_all(&d1).unwrap();
    write_file(&f1, b"hello");

    let out = run_and_stdout(&["-f", "-m", "1", "-d", "."], base);
    let ls = lines(&out);

    // dir should be present with trailing slash
    let want_dir = d1.canonicalize().unwrap().display().to_string() + "/";
    assert!(
        ls.contains(&want_dir),
        "expected to find directory '{}'\nstdout:\n{}",
        want_dir,
        out
    );

    // file should NOT be present
    let want_file = f1.canonicalize().unwrap().display().to_string();
    assert!(
        !ls.contains(&want_file),
        "did not expect file '{}'\nstdout:\n{}",
        want_file,
        out
    );

    // All lines should represent directories (trailing '/')
    assert!(
        ls.iter().all(|s| s.ends_with('/')),
        "expected all outputs to be directories with trailing '/'\nstdout:\n{}",
        out
    );
}

#[test]
fn cli_leftover_filters_to_matching_prefixes() {
    let td = tempdir().unwrap();
    let base = td.path();
    let a1 = base.join("a1");
    let a2 = base.join("a2");
    let b1 = base.join("b1");
    fs::create_dir_all(&a1).unwrap();
    fs::create_dir_all(&a2).unwrap();
    fs::create_dir_all(&b1).unwrap();
    write_file(&a1.join("f.txt"), b"x");
    write_file(&a2.join("g.txt"), b"y");
    write_file(&b1.join("h.txt"), b"z");

    // Provide PREFIX='.' and LEFTOVER='a'
    let out = run_and_stdout(&["-f", ".", "a"], base);
    let ls = lines(&out);

    let a1_abs = a1.canonicalize().unwrap();
    let a2_abs = a2.canonicalize().unwrap();
    for line in &ls {
        let p = PathBuf::from(line.trim_end_matches('/'));
        assert!(
            p.starts_with(&a1_abs) || p.starts_with(&a2_abs),
            "entry should be under a1/ or a2/: {}\nstdout:\n{}",
            line,
            out
        );
        // Must not include b1 subtree
        assert!(
            !p.starts_with(&b1),
            "should not include entries under b1: {}\nstdout:\n{}",
            line,
            out
        );
    }
}

#[test]
fn cli_max_depth_1_shows_only_direct_children() {
    let td = tempdir().unwrap();
    let base = td.path();
    let parent = base.join("parent");
    let child = parent.join("child");
    let grand = child.join("grand");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(&grand).unwrap();
    write_file(&child.join("f.txt"), b"x");
    write_file(&grand.join("g.txt"), b"y");

    let out = run_and_stdout(&["-f", "-m", "1", "."], base);
    let ls = lines(&out);

    // Only "parent" should appear among printed names, "child" and nested entries should not.
    let names: Vec<String> = ls
        .iter()
        .map(|s| {
            let p = Path::new(s.trim_end_matches('/'));
            p.file_name().unwrap().to_string_lossy().into_owned()
        })
        .collect();

    assert!(
        names.contains(&"parent".to_string()),
        "expected 'parent' to be listed at depth 1\nstdout:\n{}",
        out
    );
    assert!(
        !names.contains(&"child".to_string()),
        "did not expect 'child' at depth 1\nstdout:\n{}",
        out
    );
    assert!(
        !names.contains(&"grand".to_string()),
        "did not expect 'grand' at depth 1\nstdout:\n{}",
        out
    );
    assert!(
        !names.contains(&"f.txt".to_string())
            && !names.contains(&"g.txt".to_string()),
        "did not expect files at depth 1\nstdout:\n{}",
        out
    );
}

#[test]
fn cli_full_path_invalid_prefix_exits_with_error() {
    // Use a path that should not exist
    let invalid = "/this/definitely/should/not/exist/for/sortfs/tests";
    let mut cmd = Command::cargo_bin("sortfs").unwrap();
    let out = cmd
        .env_remove("LS_COLORS")
        .args(&["-f", invalid])
        .output()
        .unwrap();

    assert!(
        !out.status.success(),
        "expected failure for invalid path"
    );
    // Program uses process::exit(1)
    assert_eq!(out.status.code(), Some(1));
}
