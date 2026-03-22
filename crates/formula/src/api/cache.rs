use std::collections::HashMap;

use crate::types::Formula;

/// In-memory cache for formula metadata.
#[derive(Debug, Default)]
pub struct FormulaCache {
    entries: HashMap<String, Formula>,
}

impl FormulaCache {
    /// Creates an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Retrieves a formula by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Formula> {
        self.entries.get(name)
    }

    /// Inserts a formula into the cache.
    pub fn insert(&mut self, formula: Formula) {
        self.entries.insert(formula.name.clone(), formula);
    }

    /// Returns all cached formulae as a map.
    #[must_use]
    pub const fn all(&self) -> &HashMap<String, Formula> {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_formula;

    #[test]
    fn test_cache_empty_by_default() {
        let cache = FormulaCache::new();
        assert!(cache.all().is_empty());
        assert_eq!(cache.all().len(), 0);
    }

    #[test]
    fn test_cache_insert_and_get() {
        let mut cache = FormulaCache::new();
        cache.insert(test_formula("jq", &["oniguruma"]));

        assert_eq!(cache.all().len(), 1);
        assert!(!cache.all().is_empty());

        let formula = cache.get("jq");
        assert!(formula.is_some());
        assert_eq!(formula.map(|f| f.name.as_str()), Some("jq"));
    }

    #[test]
    fn test_cache_get_missing() {
        let cache = FormulaCache::new();
        assert!(cache.get("nonexistent").is_none());
    }

    #[test]
    fn test_cache_insert_multiple() {
        let mut cache = FormulaCache::new();
        for formula in [
            test_formula("a", &[]),
            test_formula("b", &["a"]),
            test_formula("c", &[]),
        ] {
            cache.insert(formula);
        }

        assert_eq!(cache.all().len(), 3);
        assert!(cache.get("a").is_some());
        assert!(cache.get("b").is_some());
        assert!(cache.get("c").is_some());
    }

    #[test]
    fn test_cache_insert_overwrites() {
        let mut cache = FormulaCache::new();
        cache.insert(test_formula("jq", &["old_dep"]));
        cache.insert(test_formula("jq", &["new_dep"]));

        assert_eq!(cache.all().len(), 1);
        let formula = cache.get("jq");
        assert_eq!(
            formula.map(|f| f.dependencies.as_slice()),
            Some(["new_dep".to_owned()].as_slice())
        );
    }

    #[test]
    fn test_cache_all() {
        let mut cache = FormulaCache::new();
        cache.insert(test_formula("a", &[]));
        cache.insert(test_formula("b", &[]));

        let all = cache.all();
        assert_eq!(all.len(), 2);
        assert!(all.contains_key("a"));
        assert!(all.contains_key("b"));
    }
}
