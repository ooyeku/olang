use anyhow::Result;
use olang::repl::Repl;

pub fn execute(verbose: bool) -> Result<()> {
    if verbose {
        println!("Starting Olang REPL (verbose mode)");
    }
    let mut repl = Repl::new(verbose)?;
    repl.run()?;
    Ok(())
}
