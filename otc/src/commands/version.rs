use anyhow::Result;

pub fn execute(verbose: bool) -> Result<()> {
    println!("Olang version: {}", olang::VERSION);
    if verbose {
        println!(
            "Toolchain binary (otc) compiled for {} on {}",
            std::env::consts::ARCH,
            std::env::consts::OS
        );
    }
    Ok(())
}
