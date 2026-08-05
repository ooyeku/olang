//! OVM Command - inspect and benchmark the bytecode tier
//!
//! Runs a program on the plain interpreter and on the tiered interpreter,
//! reporting timings and promotion statistics. The speculative execution
//! modes and optimization levels of the pre-0.24 OVM no longer exist; the
//! execution model is the interpreter plus first-call bytecode promotion.

use clap::Args;
use olang::{Interpreter, Parser};
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Args)]
pub struct OvmCommand {
    /// Input file to execute
    #[arg(value_name = "FILE")]
    pub file: PathBuf,

    /// Compare tiered execution against the plain interpreter
    #[arg(long)]
    pub compare: bool,

    /// Promotion threshold (calls before a function compiles; default 1)
    #[arg(long, default_value = "1")]
    pub threshold: u32,

    /// Show promotion statistics after execution
    #[arg(long)]
    pub stats: bool,

    /// Log each promotion decision
    #[arg(short, long)]
    pub verbose: bool,
}

impl OvmCommand {
    pub fn execute(&self) -> anyhow::Result<()> {
        let source = fs::read_to_string(&self.file)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", self.file.display(), e))?;
        let parser = Parser::new();
        let program = parser
            .parse(&source)
            .map_err(|e| anyhow::anyhow!("Parse error: {}", e))?;

        if self.compare {
            // Plain interpreter first
            let mut plain = Interpreter::new();
            plain.set_current_file(&self.file);
            let start = Instant::now();
            let plain_result = plain
                .eval_program(program.clone())
                .map_err(|e| anyhow::anyhow!("Runtime error (interpreter): {}", e))?;
            let plain_time = start.elapsed();

            // Tiered
            let mut tiered = Interpreter::new();
            tiered.enable_bytecode_tier(self.threshold, self.verbose);
            tiered.set_current_file(&self.file);
            let start = Instant::now();
            let tiered_result = tiered
                .eval_program(program)
                .map_err(|e| anyhow::anyhow!("Runtime error (tiered): {}", e))?;
            let tiered_time = start.elapsed();

            if plain_result != tiered_result {
                anyhow::bail!(
                    "DIVERGENCE: interpreter and tier disagree!\n  interpreter: {:?}\n  tiered:      {:?}",
                    plain_result,
                    tiered_result
                );
            }

            println!("Interpreter: {:?}", plain_time);
            println!("Tiered:      {:?}", tiered_time);
            if tiered_time < plain_time && !tiered_time.is_zero() {
                println!(
                    "Speedup:     {:.2}x",
                    plain_time.as_secs_f64() / tiered_time.as_secs_f64()
                );
            }
            if let Some(tier) = tiered.bytecode_tier_stats() {
                println!(
                    "Tier:        {} promoted, {} rejected, {} bytecode calls",
                    tier.promoted, tier.rejected, tier.bytecode_calls
                );
            }
        } else {
            let mut interpreter = Interpreter::new();
            interpreter.enable_bytecode_tier(self.threshold, self.verbose);
            interpreter.set_current_file(&self.file);
            let start = Instant::now();
            interpreter
                .eval_program(program)
                .map_err(|e| anyhow::anyhow!("Runtime error: {}", e))?;
            let elapsed = start.elapsed();

            println!("Executed in {:?}", elapsed);
            if self.stats {
                match interpreter.bytecode_tier_stats() {
                    Some(tier) => println!(
                        "Tier: {} promoted, {} rejected, {} bytecode calls",
                        tier.promoted, tier.rejected, tier.bytecode_calls
                    ),
                    None => println!("Tier: disabled"),
                }
            }
        }

        Ok(())
    }
}
