use std::collections::HashMap;

use brewdock_formula::{Formula, check_supportability, resolve_install_order};

#[test]
fn test_public_api_supports_supportability_and_resolution() -> Result<(), Box<dyn std::error::Error>>
{
    let jq: Formula = serde_json::from_str(include_str!("fixtures/formula/jq.json"))?;
    let oniguruma: Formula = serde_json::from_str(include_str!("fixtures/formula/oniguruma.json"))?;
    let formulae = HashMap::from([
        (jq.name.clone(), jq.clone()),
        (oniguruma.name.clone(), oniguruma),
    ]);

    check_supportability(&jq, "arm64_sequoia")?;

    let order = resolve_install_order(&formulae, &["jq"])?;
    assert_eq!(order, vec!["oniguruma", "jq"]);
    Ok(())
}
