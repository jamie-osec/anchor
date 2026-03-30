use {
    anchor_lang::{solana_program::instruction::Instruction, InstructionData, ToAccountMetas},
    anyhow::{anyhow, bail, Context, Result},
    litesvm::{types::TransactionMetadata, LiteSVM},
    paste::paste,
    serde::{Deserialize, Serialize},
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
    std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
        process::Command,
    },
};

const RESULTS_FILE: &str = "results.json";
const CURRENT_COMMIT: &str = "current";

struct ProgramSuite {
    name: &'static str,
    instructions: &'static [InstructionSuite],
}

struct InstructionSuite {
    name: &'static str,
    run: fn(&Path) -> Result<u64>,
}

// FIXME: Extend this to allow passing data to instructions
macro_rules! make_tests {
    (
        $(
            $program:ident => {
                $( $instruction:ident, )*
            },
        )*
    ) => {
        paste! {
            $(
                $(
                    fn [<run_ $program _ $instruction>](program_path: &Path) -> Result<u64> {
                        Ok(execute_instruction(
                            program_path,
                            $program::id(),
                            $program::instruction::[<$instruction:camel>] {}.data(),
                            $program::accounts::[<$instruction:camel>] {}.to_account_metas(None),
                        )?.compute_units_consumed)
                    }
                )*
            )*

            const TEST_SUITES: &[ProgramSuite] = &[
                $(
                    ProgramSuite {
                        name: stringify!($program),
                        instructions: &[
                            $(
                                InstructionSuite {
                                    name: stringify!($instruction),
                                    run: [<run_ $program _ $instruction>],
                                },
                            )*
                        ],
                    },
                )*
            ];
        }
    };
}

make_tests! {
    hello_world => {
        hello,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BenchmarkHistory {
    results: Vec<BenchmarkResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BenchmarkResult {
    commit: String,
    programs: BTreeMap<String, ProgramBenchmark>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProgramBenchmark {
    binary_size_bytes: u64,
    compute_units: BTreeMap<String, u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacyBenchmarkResults {
    programs: BTreeMap<String, LegacyProgramBenchmark>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacyProgramBenchmark {
    binary_size_bytes: u64,
    instructions: BTreeMap<String, LegacyInstructionBenchmark>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacyInstructionBenchmark {
    compute_units_consumed: u64,
}

fn main() -> Result<()> {
    let bench_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    build_programs(&bench_dir)?;
    let results_path = bench_dir.join(RESULTS_FILE);
    let current_result = build_results(&bench_dir)?;
    let mut history = load_history(&results_path)?;

    update_history(&mut history, current_result)?;

    fs::write(
        &results_path,
        format!("{}\n", serde_json::to_string_pretty(&history)?),
    )
    .with_context(|| format!("failed to write {}", results_path.display()))?;

    println!("Stored benchmark results in {}", results_path.display());

    Ok(())
}

fn build_programs(bench_dir: &Path) -> Result<()> {
    for suite in TEST_SUITES {
        let manifest_path = program_manifest_path(suite.name);
        let status = Command::new("cargo")
            .args([
                "build-sbf",
                "--manifest-path",
                &manifest_path,
                "--sbf-out-dir",
                "target/deploy",
            ])
            .current_dir(bench_dir)
            .status()
            .with_context(|| format!("failed to launch cargo build-sbf for {}", suite.name))?;

        if !status.success() {
            bail!(
                "cargo build-sbf failed for {} with status {status}",
                suite.name
            );
        }
    }

    Ok(())
}

fn execute_instruction(
    program_path: &Path,
    program_id: anchor_lang::prelude::Pubkey,
    instruction_data: Vec<u8>,
    account_metas: Vec<anchor_lang::solana_program::instruction::AccountMeta>,
) -> Result<TransactionMetadata> {
    let payer = Keypair::new();
    let mut svm = LiteSVM::new();

    svm.add_program_from_file(program_id, program_path)
        .with_context(|| format!("failed to load {}", program_path.display()))?;
    svm.airdrop(&payer.pubkey(), 1_000_000_000)
        .map_err(|failure| {
            anyhow!(
                "failed to fund benchmark payer: {:?}\n{}",
                failure.err,
                failure.meta.pretty_logs()
            )
        })?;

    let instruction = Instruction::new_with_bytes(program_id, &instruction_data, account_metas);

    let blockhash = svm.latest_blockhash();
    let message = Message::new_with_blockhash(&[instruction], Some(&payer.pubkey()), &blockhash);
    let transaction = VersionedTransaction::try_new(VersionedMessage::Legacy(message), &[&payer])
        .context("failed to create benchmark transaction")?;

    svm.send_transaction(transaction).map_err(|failure| {
        anyhow!(
            "benchmark transaction failed: {:?}\n{}",
            failure.err,
            failure.meta.pretty_logs()
        )
    })
}

fn build_results(bench_dir: &Path) -> Result<BenchmarkResult> {
    let mut programs = BTreeMap::new();

    for suite in TEST_SUITES {
        let program_path = program_binary_path(bench_dir, suite.name);
        let binary_size_bytes = fs::metadata(&program_path)
            .with_context(|| format!("failed to read metadata for {}", program_path.display()))?
            .len();
        let mut compute_units = BTreeMap::new();

        for instruction in suite.instructions {
            compute_units.insert(
                instruction.name.to_owned(),
                (instruction.run)(&program_path)?,
            );
        }

        programs.insert(
            suite.name.to_owned(),
            ProgramBenchmark {
                binary_size_bytes,
                compute_units,
            },
        );
    }

    Ok(BenchmarkResult {
        commit: CURRENT_COMMIT.to_owned(),
        programs,
    })
}

fn program_binary_path(bench_dir: &Path, program_name: &str) -> PathBuf {
    bench_dir
        .join("target/deploy")
        .join(format!("{program_name}.so"))
}

fn program_manifest_path(program_name: &str) -> String {
    format!("programs/{}/Cargo.toml", program_name.replace('_', "-"))
}

fn load_history(results_path: &Path) -> Result<BenchmarkHistory> {
    let contents = match fs::read_to_string(results_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BenchmarkHistory {
                results: Vec::new(),
            });
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", results_path.display()));
        }
    };

    if let Ok(history) = serde_json::from_str::<BenchmarkHistory>(&contents) {
        return Ok(history);
    }

    if let Ok(legacy) = serde_json::from_str::<LegacyBenchmarkResults>(&contents) {
        return Ok(BenchmarkHistory {
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
        });
    }

    Err(anyhow!(
        "failed to parse benchmark results from {}",
        results_path.display()
    ))
}

fn update_history(history: &mut BenchmarkHistory, current_result: BenchmarkResult) -> Result<()> {
    let previous_commit = previous_commit()?;
    update_history_with_previous_commit(history, current_result, &previous_commit);
    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_current_entry_in_place_when_shape_matches() {
        let mut history = BenchmarkHistory {
            results: vec![result("current", &[("hello_world", 100, &[("hello", 10)])])],
        };
        let current = result("current", &[("hello_world", 200, &[("hello", 20)])]);

        update_history_with_previous_commit(&mut history, current.clone(), "previous-sha");

        assert_eq!(history.results, vec![current]);
    }

    #[test]
    fn prepends_new_current_and_rolls_previous_entry_when_shape_changes() {
        let old_current = result("current", &[("hello_world", 100, &[("hello", 10)])]);
        let new_current = result(
            "current",
            &[("hello_world", 200, &[("hello", 20), ("hello_again", 30)])],
        );
        let mut history = BenchmarkHistory {
            results: vec![old_current.clone()],
        };

        update_history_with_previous_commit(&mut history, new_current.clone(), "previous-sha");

        assert_eq!(history.results[0], new_current);
        assert_eq!(history.results[1].commit, "previous-sha");
        assert_eq!(history.results[1].programs, old_current.programs);
    }

    #[test]
    fn turns_program_name_into_manifest_path() {
        assert_eq!(
            program_manifest_path("hello_world"),
            "programs/hello-world/Cargo.toml"
        );
    }

    fn result(commit: &str, programs: &[(&str, u64, &[(&str, u64)])]) -> BenchmarkResult {
        BenchmarkResult {
            commit: commit.to_owned(),
            programs: programs
                .iter()
                .map(|(program_name, binary_size_bytes, instructions)| {
                    (
                        (*program_name).to_owned(),
                        ProgramBenchmark {
                            binary_size_bytes: *binary_size_bytes,
                            compute_units: instructions
                                .iter()
                                .map(|(instruction_name, compute_units)| {
                                    ((*instruction_name).to_owned(), *compute_units)
                                })
                                .collect(),
                        },
                    )
                })
                .collect(),
        }
    }
}
