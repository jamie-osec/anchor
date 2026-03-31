use {
    anyhow::{anyhow, bail, Context, Result},
    serde::{Deserialize, Serialize},
    std::{collections::BTreeMap, fs, path::Path, process::Command},
};

/// File name used to persist benchmark history.
pub const RESULTS_FILE: &str = "results.json";
/// Synthetic commit label used for the latest benchmark snapshot.
pub const CURRENT_COMMIT: &str = "current";

/// Stores the benchmark history as an ordered list of snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkHistory {
    #[serde(default = "default_baseline")]
    pub baseline: BTreeMap<String, ProgramBenchmark>,
    #[serde(default = "default_baseline_programs")]
    pub baseline_programs: BTreeMap<String, Vec<String>>,
    pub results: Vec<BenchmarkResult>,
}

/// Captures benchmark results for a single commit or synthetic snapshot label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub commit: String,
    pub programs: BTreeMap<String, ProgramBenchmark>,
}

/// Records the measured output for one benchmarked program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramBenchmark {
    pub binary_size_bytes: u64,
    pub compute_units: BTreeMap<String, u64>,
}

/// Legacy flat benchmark payload kept for migration from older result files.
#[derive(Debug, Serialize, Deserialize)]
struct LegacyBenchmarkResults {
    programs: BTreeMap<String, LegacyProgramBenchmark>,
}

/// Legacy per-program benchmark format used before history support.
#[derive(Debug, Serialize, Deserialize)]
struct LegacyProgramBenchmark {
    binary_size_bytes: u64,
    instructions: BTreeMap<String, LegacyInstructionBenchmark>,
}

/// Legacy per-instruction metric format used before flattening compute-unit fields.
#[derive(Debug, Serialize, Deserialize)]
struct LegacyInstructionBenchmark {
    compute_units_consumed: u64,
}

/// Loads benchmark history from disk, migrating older JSON formats when needed.
pub fn load_history(results_path: &Path) -> Result<BenchmarkHistory> {
    let contents = match fs::read_to_string(results_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BenchmarkHistory {
                baseline: default_baseline(),
                baseline_programs: default_baseline_programs(),
                results: Vec::new(),
            });
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", results_path.display()));
        }
    };

    if let Ok(history) = serde_json::from_str::<BenchmarkHistory>(&contents) {
        return Ok(normalize_history(history));
    }

    if let Ok(legacy) = serde_json::from_str::<LegacyBenchmarkResults>(&contents) {
        return Ok(normalize_history(BenchmarkHistory {
            baseline: default_baseline(),
            baseline_programs: default_baseline_programs(),
            results: vec![BenchmarkResult {
                commit: CURRENT_COMMIT.to_owned(),
                programs: legacy
                    .programs
                    .into_iter()
                    .map(|(program_name, program)| {
                        (
                            program_name,
                            ProgramBenchmark {
                                binary_size_bytes: program.binary_size_bytes,
                                compute_units: program
                                    .instructions
                                    .into_iter()
                                    .map(|(instruction_name, instruction)| {
                                        (instruction_name, instruction.compute_units_consumed)
                                    })
                                    .collect(),
                            },
                        )
                    })
                    .collect(),
            }],
        }));
    }

    Err(anyhow!(
        "failed to parse benchmark results from {}",
        results_path.display()
    ))
}

/// Writes the current benchmark history to disk using pretty-printed JSON.
pub fn save_history(results_path: &Path, history: &BenchmarkHistory) -> Result<()> {
    fs::write(
        results_path,
        format!("{}\n", serde_json::to_string_pretty(history)?),
    )
    .with_context(|| format!("failed to write {}", results_path.display()))
}

/// Inserts the latest benchmark snapshot into history using the previous git commit when needed.
pub fn update_history(
    history: &mut BenchmarkHistory,
    current_result: BenchmarkResult,
) -> Result<()> {
    if history.baseline.is_empty() {
        history.baseline = default_baseline();
    }
    if history.baseline_programs.is_empty() {
        history.baseline_programs = default_baseline_programs();
    }
    let previous_commit = previous_commit()?;
    update_history_with_previous_commit(history, current_result, &previous_commit);
    Ok(())
}

/// Returns a placeholder baseline payload until another framework is wired in.
fn default_baseline() -> BTreeMap<String, ProgramBenchmark> {
    [
        (
            "hello_world_quasar".to_owned(),
            ProgramBenchmark {
                binary_size_bytes: 0,
                compute_units: [("hello".to_owned(), 0)].into_iter().collect(),
            },
        ),
        (
            "hello_world_other_framework_name".to_owned(),
            ProgramBenchmark {
                binary_size_bytes: 0,
                compute_units: [("hello".to_owned(), 0)].into_iter().collect(),
            },
        ),
        (
            "multisig_quasar".to_owned(),
            ProgramBenchmark {
                binary_size_bytes: 0,
                compute_units: [
                    ("create".to_owned(), 0),
                    ("deposit".to_owned(), 0),
                    ("execute_transfer".to_owned(), 0),
                    ("set_label".to_owned(), 0),
                ]
                .into_iter()
                .collect(),
            },
        ),
    ]
    .into_iter()
    .collect()
}

/// Returns the configured baseline-program comparison list for each benchmarked program.
fn default_baseline_programs() -> BTreeMap<String, Vec<String>> {
    [
        (
            "hello_world".to_owned(),
            vec![
                "hello_world_quasar".to_owned(),
                "hello_world_other_framework_name".to_owned(),
            ],
        ),
        ("multisig".to_owned(), vec!["multisig_quasar".to_owned()]),
    ]
    .into_iter()
    .collect()
}

/// Normalizes persisted history to the current benchmark naming scheme.
fn normalize_history(mut history: BenchmarkHistory) -> BenchmarkHistory {
    normalize_benchmark_program_map(&mut history.baseline_programs);

    for result in &mut history.results {
        normalize_result_program_map(&mut result.programs);
    }

    history
}

/// Applies benchmark program renames within a current-results map.
fn normalize_result_program_map(programs: &mut BTreeMap<String, ProgramBenchmark>) {
    if let Some(multisig_quasar) = programs.remove("multisig_quasar") {
        programs.insert("multisig".to_owned(), multisig_quasar);
    }
}

/// Applies benchmark program renames within the baseline comparison mapping.
fn normalize_benchmark_program_map(programs: &mut BTreeMap<String, Vec<String>>) {
    if let Some(multisig_quasar) = programs.remove("multisig_quasar") {
        programs.insert("multisig".to_owned(), multisig_quasar);
    }
}

/// Updates the history with a caller-provided previous commit reference.
fn update_history_with_previous_commit(
    history: &mut BenchmarkHistory,
    current_result: BenchmarkResult,
    previous_commit: &str,
) {
    match history.results.first_mut() {
        Some(existing_current) if existing_current.commit == CURRENT_COMMIT => {
            if benchmark_shape_changed(existing_current, &current_result) {
                existing_current.commit = previous_commit.to_owned();
                history.results.insert(0, current_result);
            } else {
                *existing_current = current_result;
            }
        }
        _ => history.results.insert(0, current_result),
    }
}

/// Returns true when the set of benchmarked programs or instructions has changed.
fn benchmark_shape_changed(previous: &BenchmarkResult, current: &BenchmarkResult) -> bool {
    previous.programs.keys().ne(current.programs.keys())
        || previous
            .programs
            .iter()
            .any(|(program_name, previous_program)| {
                let Some(current_program) = current.programs.get(program_name) else {
                    return true;
                };

                previous_program
                    .compute_units
                    .keys()
                    .ne(current_program.compute_units.keys())
            })
}

/// Resolves the commit immediately before `HEAD` for history rollover.
fn previous_commit() -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD~1"])
        .output()
        .context("failed to get previous commit")?;

    if !output.status.success() {
        bail!(
            "failed to get previous commit: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(String::from_utf8(output.stdout)
        .context("previous commit was not valid UTF-8")?
        .trim()
        .to_owned())
}
