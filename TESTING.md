# sortfs Testing Guide

This project includes a rich set of unit and integration tests to validate core behavior of the filesystem sorter CLI.

Test Strategy

- Unit tests (in src/main.rs)
  - print_path formatting (directory trailing slash behavior)
  - build_entries sorting by mtime (DESC) with deterministic tie-break by path (ASC)
  - build_entries filters:
    - dirs_only
    - leftover prefix filter (only traverse subtrees whose top-level names start with the given prefix)
    - max_depth handling (e.g., depth=1 shows only direct children)
  - normalize_path negative case (invalid path yields error)

- Integration tests (in tests/cli.rs)
  - End-to-end CLI invocations using assert_cmd
  - Scenarios:
    - -f -m 1 .: depth-limited listing with full paths
    - -d directories-only mode excludes files and adds a trailing slash
    - leftover filter via positional args keeps only matching prefix subtrees
    - max-depth=1 ensures only top-level children are listed
    - invalid -f prefix causes a clean exit(1)
  - Implementation notes:
    - All tests isolate file-system state using tempfile
    - LS_COLORS is removed for deterministic, non-colored output unless explicitly testing color
    - File mtimes are controlled using filetime for stable ordering

How to run tests

- Run all tests:
  - cargo test

- Run only integration tests:
  - cargo test --test cli

- Run only unit tests:
  - cargo test --bin sortfs

- Run a single test by name:
  - cargo test test_build_entries_dirs_only

- Show stdout/stderr during a failing test:
  - cargo test -- --nocapture

Notes and future coverage ideas

- Follow-links behavior: add cases with symlinks to ensure traversal matches expectations.
- Hidden files and ignore files: add tests for .fdignore and .git/ exclusions to ensure filters are respected.
- Colorized output (-c): add a targeted test with a controlled LS_COLORS to assert presence of ANSI sequences without relying on exact codes.
- Error paths: simulate write failures on stdout (e.g., broken pipe) and assert exit code handling.
- Feature flags: add coverage for gnu_legacy compatibility if the feature is used in downstream environments.
