use {
    crate::history::{BenchmarkResult, ProgramBenchmark, CURRENT_COMMIT},
    anchor_lang::{
        prelude::Pubkey,
        solana_program::instruction::{AccountMeta, Instruction},
    },
    anyhow::{anyhow, bail, Context, Result},
    litesvm::{types::TransactionMetadata, LiteSVM},
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

/// Describes a benchmarked program and the instruction cases it exposes.
pub struct ProgramSuite {
    pub name: &'static str,
    pub instructions: &'static [InstructionSuite],
}

/// Describes a single instruction benchmark within a program suite.
pub struct InstructionSuite {
    pub name: &'static str,
    pub run: fn(&Path) -> Result<TransactionMetadata>,
}

/// Builds a ready-to-run benchmark transaction for a program instruction.
pub type CaseBuilder = fn(&mut BenchContext) -> Result<BenchInstruction>;

/// Holds the LiteSVM instance and shared payer used for a single benchmark run.
pub struct BenchContext {
    payer: Keypair,
    program_id: Pubkey,
    svm: LiteSVM,
}

/// Represents one benchmark transaction plus any additional required signers.
pub struct BenchInstruction {
    instruction_data: Vec<u8>,
    account_metas: Vec<AccountMeta>,
    signers: Vec<Keypair>,
}

impl BenchInstruction {
    /// Creates a benchmark instruction from serialized data and account metas.
    pub fn new(instruction_data: Vec<u8>, account_metas: Vec<AccountMeta>) -> Self {
        Self {
            instruction_data,
            account_metas,
            signers: Vec::new(),
        }
    }

    /// Adds a single extra signer to the benchmark transaction.
    pub fn with_signer(mut self, signer: Keypair) -> Self {
        self.signers.push(signer);
        self
    }

    /// Adds multiple extra signers to the benchmark transaction.
    pub fn with_signers(mut self, signers: Vec<Keypair>) -> Self {
        self.signers.extend(signers);
        self
    }
}

impl BenchContext {
    /// Creates a fresh LiteSVM instance with the target program loaded and a funded payer.
    pub fn new(program_path: &Path, program_id: Pubkey) -> Result<Self> {
        let payer = keypair_for_account("bench-payer");
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

        Ok(Self {
            payer,
            program_id,
            svm,
        })
    }

    /// Funds an account inside the benchmark VM before running an instruction.
    pub fn airdrop(&mut self, pubkey: &Pubkey, lamports: u64) -> Result<()> {
        self.svm.airdrop(pubkey, lamports).map_err(|failure| {
            anyhow!(
                "failed to fund benchmark account {}: {:?}\n{}",
                pubkey,
                failure.err,
                failure.meta.pretty_logs()
            )
        })?;
        Ok(())
    }

    /// Executes a benchmark instruction with any signers attached to it.
    pub fn execute(&mut self, instruction: BenchInstruction) -> Result<TransactionMetadata> {
        let signer_refs = instruction
            .signers
            .iter()
            .map(|signer| signer as &dyn solana_signer::Signer)
            .collect::<Vec<_>>();

        self.execute_raw(
            instruction.instruction_data,
            instruction.account_metas,
            &signer_refs,
        )
    }

    /// Executes a benchmark instruction using an explicit signer list.
    pub fn execute_with_signers(
        &mut self,
        instruction_data: Vec<u8>,
        account_metas: Vec<AccountMeta>,
        signers: &[&dyn solana_signer::Signer],
    ) -> Result<TransactionMetadata> {
        self.execute_raw(instruction_data, account_metas, signers)
    }

    /// Constructs and submits the underlying transaction to LiteSVM.
    fn execute_raw(
        &mut self,
        instruction_data: Vec<u8>,
        account_metas: Vec<AccountMeta>,
        signers: &[&dyn solana_signer::Signer],
    ) -> Result<TransactionMetadata> {
        let instruction =
            Instruction::new_with_bytes(self.program_id, &instruction_data, account_metas);

        let blockhash = self.svm.latest_blockhash();
        let message =
            Message::new_with_blockhash(&[instruction], Some(&self.payer.pubkey()), &blockhash);
        let mut all_signers: Vec<&dyn solana_signer::Signer> = vec![&self.payer];
        all_signers.extend_from_slice(signers);
        let transaction =
            VersionedTransaction::try_new(VersionedMessage::Legacy(message), &all_signers)
                .context("failed to create benchmark transaction")?;

        self.svm.send_transaction(transaction).map_err(|failure| {
            anyhow!(
                "benchmark transaction failed: {:?}\n{}",
                failure.err,
                failure.meta.pretty_logs()
            )
        })
    }
}

/// Builds every benchmarked program into `target/deploy` before measurement.
pub fn build_programs(bench_dir: &Path, suites: &[ProgramSuite]) -> Result<()> {
    for suite in suites {
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

/// Loads a program into LiteSVM, prepares a case, and executes the benchmark transaction.
pub fn execute_benchmark(
    program_path: &Path,
    program_id: Pubkey,
    case_builder: CaseBuilder,
) -> Result<TransactionMetadata> {
    let mut ctx = BenchContext::new(program_path, program_id)?;
    let instruction = case_builder(&mut ctx)?;
    ctx.execute(instruction)
}

/// Collects binary size and compute-unit measurements for all configured suites.
pub fn build_results(bench_dir: &Path, suites: &[ProgramSuite]) -> Result<BenchmarkResult> {
    let mut programs = BTreeMap::new();

    for suite in suites {
        let program_path = program_binary_path(bench_dir, suite.name);
        let binary_size_bytes = fs::metadata(&program_path)
            .with_context(|| format!("failed to read metadata for {}", program_path.display()))?
            .len();
        let mut compute_units = BTreeMap::new();

        for instruction in suite.instructions {
            let metadata = (instruction.run)(&program_path)?;
            compute_units.insert(instruction.name.to_owned(), metadata.compute_units_consumed);
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

/// Derives a stable keypair from an account label so benchmark inputs are repeatable.
pub fn keypair_for_account(name: &str) -> Keypair {
    let mut seed = [0u8; 32];

    for (index, byte) in name.bytes().enumerate() {
        let position = index % seed.len();
        seed[position] = seed[position]
            .wrapping_mul(31)
            .wrapping_add(byte)
            .wrapping_add(index as u8);
    }

    Keypair::new_from_array(seed)
}

/// Returns the expected on-disk path for a compiled program binary.
fn program_binary_path(bench_dir: &Path, program_name: &str) -> PathBuf {
    bench_dir
        .join("target/deploy")
        .join(format!("{program_name}.so"))
}

/// Returns the Cargo manifest path for a program based on its suite name.
fn program_manifest_path(program_name: &str) -> String {
    format!("programs/{}/Cargo.toml", program_name.replace('_', "-"))
}
