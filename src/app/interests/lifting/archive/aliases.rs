//! Pure alias resolution shared by database writes and the immutable
//! snapshot. Database/admin writes keep rows direct, but reads follow a
//! bounded chain defensively so a hand-edited table cannot loop forever.

use std::collections::{HashMap, HashSet};

use benjisponge::data::fitness_models::ExerciseAlias;

#[derive(Clone, Debug, Default)]
pub struct AliasMap {
    direct: HashMap<String, String>,
}

impl AliasMap {
    pub fn new(rows: impl IntoIterator<Item = ExerciseAlias>) -> Self {
        let direct = rows
            .into_iter()
            .filter(|row| row.alias_name != row.canonical_name)
            .map(|row| (row.alias_name, row.canonical_name))
            .collect();
        Self { direct }
    }

    pub fn is_empty(&self) -> bool {
        self.direct.is_empty()
    }

    /// Resolve an exact stored/imported name. Cycles are invalid application
    /// state; a defensive fallback to the original name keeps the archive
    /// readable if one is introduced through direct database editing.
    pub fn resolve(&self, name: &str) -> String {
        let mut current = name;
        let mut seen = HashSet::new();
        while let Some(next) = self.direct.get(current) {
            if !seen.insert(current) || seen.contains(next.as_str()) {
                return name.to_string();
            }
            current = next;
        }
        current.to_string()
    }

    /// Exact exercise filters historically compare ASCII case-insensitively.
    /// Preserve that behavior for old alias-valued links too.
    pub fn resolve_filter(&self, name: &str) -> String {
        if self.direct.contains_key(name) {
            return self.resolve(name);
        }
        self.direct
            .keys()
            .find(|alias| alias.eq_ignore_ascii_case(name))
            .map_or_else(|| name.to_string(), |alias| self.resolve(alias))
    }

    pub fn aliases_for(&self, canonical_name: &str) -> Vec<String> {
        let mut aliases: Vec<String> = self
            .direct
            .keys()
            .filter(|alias| {
                alias.as_str() != canonical_name && self.resolve(alias) == canonical_name
            })
            .cloned()
            .collect();
        aliases.sort_unstable_by(|a, b| {
            a.to_ascii_lowercase()
                .cmp(&b.to_ascii_lowercase())
                .then_with(|| a.cmp(b))
        });
        aliases
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(alias: &str, canonical: &str) -> ExerciseAlias {
        ExerciseAlias {
            alias_name: alias.into(),
            canonical_name: canonical.into(),
        }
    }

    #[test]
    fn resolves_direct_rows_chains_and_filter_case() {
        let aliases = AliasMap::new([
            row("Military Press", "Barbell Overhead Press"),
            row("Barbell Overhead Press", "Standing Press"),
        ]);
        assert_eq!(aliases.resolve("Military Press"), "Standing Press");
        assert_eq!(aliases.resolve_filter("military press"), "Standing Press");
        assert_eq!(
            aliases.aliases_for("Standing Press"),
            vec!["Barbell Overhead Press", "Military Press"]
        );
    }

    #[test]
    fn a_direct_database_cycle_falls_back_without_hanging() {
        let aliases = AliasMap::new([row("A", "B"), row("B", "A")]);
        assert_eq!(aliases.resolve("A"), "A");
        assert_eq!(aliases.resolve("B"), "B");
        assert!(aliases.aliases_for("A").is_empty());
    }
}
