use std::collections::HashMap;

use crate::{
    error::{DependencyCycle, FormulaError},
    types::Formula,
};

/// Resolves the install order for the given formulae, including all transitive
/// dependencies.
///
/// Returns formula names in topological order (dependencies first).
///
/// # Errors
///
/// Returns [`FormulaError::CyclicDependency`] if a dependency cycle is detected.
/// Returns [`FormulaError::NotFound`] if a dependency is missing from the map.
pub fn resolve_install_order<S: ::std::hash::BuildHasher>(
    formulae: &HashMap<String, Formula, S>,
    requested: &[String],
) -> Result<Vec<String>, FormulaError> {
    let mut state: HashMap<&str, VisitState> = HashMap::new();
    let mut result = Vec::new();
    let mut path = Vec::new();

    for name in requested {
        visit(name, formulae, &mut state, &mut result, &mut path)?;
    }

    Ok(result)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VisitState {
    InProgress,
    Done,
}

fn visit<'a, S: ::std::hash::BuildHasher>(
    name: &'a str,
    formulae: &'a HashMap<String, Formula, S>,
    state: &mut HashMap<&'a str, VisitState>,
    result: &mut Vec<String>,
    path: &mut Vec<String>,
) -> Result<(), FormulaError> {
    match state.get(name) {
        Some(VisitState::Done) => return Ok(()),
        Some(VisitState::InProgress) => {
            let mut cycle = path
                .iter()
                .position(|n| n == name)
                .map_or_else(Vec::new, |start| path[start..].to_vec());
            cycle.push(name.to_owned());
            return Err(FormulaError::CyclicDependency(DependencyCycle::new(cycle)));
        }
        None => {}
    }

    let formula = formulae.get(name).ok_or_else(|| FormulaError::NotFound {
        name: name.to_owned(),
    })?;

    state.insert(name, VisitState::InProgress);
    path.push(name.to_owned());

    for dep in &formula.dependencies {
        visit(dep, formulae, state, result, path)?;
    }

    path.pop();
    state.insert(name, VisitState::Done);
    result.push(name.to_owned());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::test_formula;

    fn make_map(formulae: Vec<Formula>) -> HashMap<String, Formula> {
        formulae.into_iter().map(|f| (f.name.clone(), f)).collect()
    }

    #[test]
    fn test_resolve_no_deps() -> Result<(), FormulaError> {
        let formulae = make_map(vec![test_formula("a", &[])]);
        let order = resolve_install_order(&formulae, &["a".to_owned()])?;
        assert_eq!(order, vec!["a"]);
        Ok(())
    }

    #[test]
    fn test_resolve_linear_deps() -> Result<(), FormulaError> {
        // a -> b -> c
        let formulae = make_map(vec![
            test_formula("a", &["b"]),
            test_formula("b", &["c"]),
            test_formula("c", &[]),
        ]);
        let order = resolve_install_order(&formulae, &["a".to_owned()])?;
        assert_eq!(order, vec!["c", "b", "a"]);
        Ok(())
    }

    #[test]
    fn test_resolve_diamond_deps() -> Result<(), FormulaError> {
        // a -> b, a -> c, b -> d, c -> d
        let formulae = make_map(vec![
            test_formula("a", &["b", "c"]),
            test_formula("b", &["d"]),
            test_formula("c", &["d"]),
            test_formula("d", &[]),
        ]);
        let order = resolve_install_order(&formulae, &["a".to_owned()])?;

        // d must come before b and c; b and c before a
        let pos = |name: &str| order.iter().position(|n| n == name);
        assert!(pos("d") < pos("b"));
        assert!(pos("d") < pos("c"));
        assert!(pos("b") < pos("a"));
        assert!(pos("c") < pos("a"));

        // d appears only once
        assert_eq!(order.iter().filter(|n| *n == "d").count(), 1);
        Ok(())
    }

    #[test]
    fn test_resolve_cycle_detected() {
        // a -> b -> c -> a
        let formulae = make_map(vec![
            test_formula("a", &["b"]),
            test_formula("b", &["c"]),
            test_formula("c", &["a"]),
        ]);
        let err = resolve_install_order(&formulae, &["a".to_owned()]);
        assert!(matches!(err, Err(FormulaError::CyclicDependency(_))));
    }

    #[test]
    fn test_resolve_self_cycle() {
        // a -> a
        let formulae = make_map(vec![test_formula("a", &["a"])]);
        let err = resolve_install_order(&formulae, &["a".to_owned()]);
        assert!(matches!(err, Err(FormulaError::CyclicDependency(_))));
    }

    #[test]
    fn test_resolve_missing_dependency() {
        let formulae = make_map(vec![test_formula("a", &["missing"])]);
        let err = resolve_install_order(&formulae, &["a".to_owned()]);
        assert!(matches!(err, Err(FormulaError::NotFound { .. })));
    }

    #[test]
    fn test_resolve_multiple_requested() -> Result<(), FormulaError> {
        // Request both a and b independently
        let formulae = make_map(vec![
            test_formula("a", &["c"]),
            test_formula("b", &["c"]),
            test_formula("c", &[]),
        ]);
        let order = resolve_install_order(&formulae, &["a".to_owned(), "b".to_owned()])?;

        // c appears once, before both a and b
        let pos = |name: &str| order.iter().position(|n| n == name);
        assert!(pos("c") < pos("a"));
        assert!(pos("c") < pos("b"));
        assert_eq!(order.len(), 3);
        Ok(())
    }

    #[test]
    fn test_resolve_already_in_order() -> Result<(), FormulaError> {
        // Requesting a formula twice should not duplicate
        let formulae = make_map(vec![test_formula("a", &[])]);
        let order = resolve_install_order(&formulae, &["a".to_owned(), "a".to_owned()])?;
        assert_eq!(order, vec!["a"]);
        Ok(())
    }
}
