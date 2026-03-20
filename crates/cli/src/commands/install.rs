use anyhow::Result;

#[allow(clippy::unnecessary_wraps)]
pub fn run(formulae: &[String]) -> Result<()> {
    for formula in formulae {
        println!("install: {formula} (stub)");
    }
    Ok(())
}
