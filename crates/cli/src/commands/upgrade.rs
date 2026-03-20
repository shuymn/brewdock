use anyhow::Result;

#[allow(clippy::unnecessary_wraps)]
pub fn run(formulae: &[String]) -> Result<()> {
    if formulae.is_empty() {
        println!("upgrade: all (stub)");
    } else {
        for formula in formulae {
            println!("upgrade: {formula} (stub)");
        }
    }
    Ok(())
}
