mod bench;
mod history;
mod programs;

use {
    crate::{
        bench::{
            build_programs, build_results, execute_benchmark, BenchContext, BenchInstruction,
            InstructionSuite, ProgramSuite,
        },
        history::{load_history, save_history, update_history, RESULTS_FILE},
    },
    anchor_lang::{InstructionData, ToAccountMetas},
    anyhow::Result,
    litesvm::types::TransactionMetadata,
    paste::paste,
    std::path::{Path, PathBuf},
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

/// Builds benchmark programs, runs the configured suites, and updates `results.json`.
fn main() -> Result<()> {
    let bench_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    build_programs(&bench_dir, TEST_SUITES)?;
    let results_path = bench_dir.join(RESULTS_FILE);
    let current_result = build_results(&bench_dir, TEST_SUITES)?;
    let mut history = load_history(&results_path)?;

    update_history(&mut history, current_result)?;
    save_history(&results_path, &history)?;

    println!("Stored benchmark results in {}", results_path.display());

    Ok(())
}
