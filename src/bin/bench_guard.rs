use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = env::args().collect();
    let requested = args.get(1).map(String::as_str).unwrap_or("target/criterion");

    let root = resolve_criterion_root(requested).unwrap_or_else(|| {
        eprintln!(
            "bench_guard: criterion dir not found: {requested} (also checked parent target dirs)"
        );
        std::process::exit(2);
    });

    // Absolute thresholds (ns). These are set high to avoid CI flakiness while still
    // catching major regressions. They can be tightened once CI variance is known.
    let budgets = [
        ("phase2/reflex_assert_force", 250_000u64),  // 250µs
        ("phase2/reflex_resolve_simple", 50_000u64), // 50µs
    ];

    let mut failures = Vec::new();
    for (bench_name, max_median_ns) in budgets {
        match read_median_ns(&root, bench_name) {
            Ok(median_ns) => {
                if median_ns > max_median_ns {
                    failures.push(format!(
                        "{bench_name}: median {median_ns}ns > budget {max_median_ns}ns"
                    ));
                } else {
                    println!("{bench_name}: median {median_ns}ns (budget {max_median_ns}ns)");
                }
            }
            Err(err) => {
                failures.push(format!("{bench_name}: failed to read median: {err}"));
            }
        }
    }

    if failures.is_empty() {
        return;
    }

    eprintln!("bench_guard failed:");
    for f in failures {
        eprintln!("- {f}");
    }
    std::process::exit(1);
}

fn resolve_criterion_root(requested: &str) -> Option<PathBuf> {
    let candidate = PathBuf::from(requested);
    if candidate.exists() {
        return Some(candidate);
    }

    let requested_path = Path::new(requested);
    // Absolute paths are already handled by the `exists()` check above.
    // If an absolute path doesn't exist, walking parents won't help.
    if requested_path.is_absolute() {
        return None;
    }

    // Some environments (workspaces, CI) configure Cargo to place `target/` outside
    // the crate directory. Walk upwards and look for the requested relative path.
    let mut dir = env::current_dir().ok()?;
    for _ in 0..6 {
        let c = dir.join(requested_path);
        if c.exists() {
            return Some(c);
        }
        if !dir.pop() {
            break;
        }
    }

    None
}

fn read_median_ns(root: &Path, bench_name: &str) -> Result<u64, String> {
    let estimates = find_estimates_json(root, bench_name)
        .ok_or_else(|| format!("could not locate estimates.json for {bench_name}"))?;

    let bytes = fs::read(&estimates).map_err(|e| format!("read {}: {e}", estimates.display()))?;
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", estimates.display()))?;

    let median = json
        .get("median")
        .and_then(|v| v.get("point_estimate"))
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| format!("missing median.point_estimate in {}", estimates.display()))?;

    if !median.is_finite() || median < 0.0 {
        return Err(format!("invalid median value {median} in {}", estimates.display()));
    }

    Ok(median.round() as u64)
}

fn find_estimates_json(root: &Path, bench_name: &str) -> Option<PathBuf> {
    // Criterion path structure is typically:
    // target/criterion/<group>/<bench_id>/new/estimates.json
    // We search for a directory whose relative path contains the benchmark name tokens.
    let normalized = bench_name.replace('\\', "/");
    let tokens: Vec<&str> = normalized.split('/').filter(|t| !t.is_empty()).collect();

    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            // Skip unreadable directories instead of aborting the entire search.
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }

            if path.file_name().and_then(|n| n.to_str()) != Some("estimates.json") {
                continue;
            }

            let rel = path.strip_prefix(root).ok()?;
            let rel_s = rel.to_string_lossy().replace('\\', "/");

            // Heuristic: must be under a "new/estimates.json" directory.
            if !rel_s.ends_with("/new/estimates.json") {
                continue;
            }

            if tokens.iter().all(|t| rel_s.contains(t)) {
                return Some(path);
            }
        }
    }
    None
}
