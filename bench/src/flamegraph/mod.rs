mod ix_trace;
mod svg;
mod trace;

use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

pub struct FlamegraphReport {
    pub program_name: String,
    pub total_cu: u64,
    pub stacks: BTreeMap<Vec<String>, u64>,
}

/// Re-export the per-instruction trace printer so the top-level crate can
/// invoke it from the bench harness.
pub fn print_ix_trace_to<W: Write>(
    writer: &mut W,
    label: &str,
    elf_path: &Path,
    trace_dir: &Path,
    manifest_dir: Option<&Path>,
) -> Result<()> {
    ix_trace::print_trace_to(writer, label, elf_path, trace_dir, manifest_dir)
}

/// Generates a flamegraph SVG from LiteSVM register trace files.
///
/// `trace_dir` should contain the `.regs` and `.insns` files produced by
/// running a transaction with `LiteSVM::new_debuggable(true)`.
///
/// `manifest_dir` is an optional pointer to the program's Cargo manifest
/// directory — when supplied, symbol lookup prefers the unstripped binary
/// inside that workspace's target tree (avoiding ambiguity when the same
/// lib name appears in multiple workspaces).
pub fn generate_flamegraph_from_trace(
    program_name: &str,
    elf_path: &Path,
    trace_dir: &Path,
    output_path: &Path,
    manifest_dir: Option<&Path>,
) -> Result<()> {
    let report = trace::build_report_from_trace(program_name, elf_path, trace_dir, manifest_dir)?;
    let Some(report) = report else {
        return Ok(());
    };
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_path, svg::render(&report))?;
    Ok(())
}
