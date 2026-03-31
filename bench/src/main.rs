mod bench;
mod graphs;
mod history;
mod programs;

use {
    crate::{
        bench::{
            build_programs, build_results, execute_benchmark, BenchContext, BenchInstruction,
            InstructionSuite, ProgramSuite,
        },
        graphs::{render_graphs, GRAPHS_DIR},
        history::{
            load_history, save_history, update_history, validate_history_commits, RESULTS_FILE,
        },
    },
    anchor_lang::{InstructionData, ToAccountMetas},
    anyhow::{bail, Result},
    litesvm::types::TransactionMetadata,
    paste::paste,
    std::{
        env,
        path::{Path, PathBuf},
    },
};

macro_rules! make_tests {
    (
        $(
            $program:ident => {
                $( $instruction:ident $(=> $builder:path)?, )*
            },
        )*
    ) => {
        paste! {
            $(
                $(
                    make_tests!(@runner $program, $instruction $(, $builder)?);
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
    (@runner $program:ident, $instruction:ident) => {
        paste! {
            fn [<build_ $program _ $instruction _case>](
                _ctx: &mut BenchContext,
            ) -> Result<BenchInstruction> {
                Ok(BenchInstruction::new(
                    $program::instruction::[<$instruction:camel>] {}.data(),
                    $program::accounts::[<$instruction:camel>] {}.to_account_metas(None),
                ))
            }

            fn [<run_ $program _ $instruction>](program_path: &Path) -> Result<TransactionMetadata> {
                execute_benchmark(
                    program_path,
                    $program::id(),
                    [<build_ $program _ $instruction _case>],
                )
            }
        }
    };
    (@runner $program:ident, $instruction:ident, $custom_builder:path) => {
        paste! {
            fn [<run_ $program _ $instruction>](program_path: &Path) -> Result<TransactionMetadata> {
                execute_benchmark(program_path, $program::id(), $custom_builder)
            }
        }
    };
}

make_tests! {
    hello_world => {
        hello,
    },
    multisig => {
        create => programs::multisig::build_create_case,
        deposit => programs::multisig::build_deposit_case,
        set_label => programs::multisig::build_set_label_case,
        execute_transfer => programs::multisig::build_execute_transfer_case,
    },
}

/// Controls whether the benchmark run updates the history file or only validates it.
enum RunMode {
    Record,
    Check,
}

impl RunMode {
    /// Parses the requested benchmark run mode from the CLI arguments.
    fn from_args() -> Result<Self> {
        match env::args().nth(1).as_deref() {
            None => Ok(Self::Record),
            Some("check" | "--check") => Ok(Self::Check),
            Some(argument) => bail!("unsupported anchor-bench mode: {argument}"),
        }
    }
}

/// Builds benchmark programs, runs the configured suites, and updates `results.json`.
fn main() -> Result<()> {
    let mode = RunMode::from_args()?;
    let bench_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    build_programs(&bench_dir, TEST_SUITES)?;
    let results_path = bench_dir.join(RESULTS_FILE);
    let current_result = build_results(&bench_dir, TEST_SUITES)?;
    let history = load_history(&results_path)?;
    let mut updated_history = history.clone();

    update_history(&mut updated_history, current_result)?;
    validate_history_commits(&updated_history)?;

    match mode {
        RunMode::Record => {
            save_history(&results_path, &updated_history)?;
            render_graphs(&bench_dir, &updated_history)?;

            println!("Stored benchmark results in {}", results_path.display());
            println!(
                "Stored benchmark graphs in {}",
                bench_dir.join(GRAPHS_DIR).display()
            );
        }
        RunMode::Check => {
            if history != updated_history {
                bail!(
                    "benchmarks have changed without being recorded in {}. Run `cargo run \
                     --manifest-path bench/Cargo.toml --locked` to refresh them.",
                    results_path.display()
                );
            }

            println!("Benchmark history is up to date");
        }
    }

    Ok(())
}
