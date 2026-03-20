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

    /// Inserts all formulae from an iterator.
    pub fn insert_all(&mut self, formulae: impl IntoIterator<Item = Formula>) {
        let iter = formulae.into_iter();
        self.entries.reserve(iter.size_hint().0);
        for formula in iter {
            self.insert(formula);
        }
    }

    /// Returns all cached formulae as a map.
    #[must_use]
    pub const fn all(&self) -> &HashMap<String, Formula> {
        &self.entries
    }

    /// Returns the number of cached formulae.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::test_formula;

    #[test]
    fn test_cache_empty_by_default() {
        let cache = FormulaCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_cache_insert_and_get() {
        let mut cache = FormulaCache::new();
        cache.insert(test_formula("jq", &["oniguruma"]));

        assert_eq!(cache.len(), 1);
        assert!(!cache.is_empty());

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
    fn test_cache_insert_all() {
        let mut cache = FormulaCache::new();
        cache.insert_all(vec![
            test_formula("a", &[]),
            test_formula("b", &["a"]),
            test_formula("c", &[]),
        ]);

        assert_eq!(cache.len(), 3);
        assert!(cache.get("a").is_some());
        assert!(cache.get("b").is_some());
        assert!(cache.get("c").is_some());
    }

    #[test]
    fn test_cache_insert_overwrites() {
        let mut cache = FormulaCache::new();
        cache.insert(test_formula("jq", &["old_dep"]));
        cache.insert(test_formula("jq", &["new_dep"]));

        assert_eq!(cache.len(), 1);
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
