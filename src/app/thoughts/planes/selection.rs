//! A selection removes each (bar, slice) at most once, even when independent
//! cut ladders overlap. Totals, combined segments, and erased styling all use
//! this same union.

use std::collections::HashSet;

use super::reference_data::{CutOption, CutSlice, SacrificeBar};

pub struct SelectedSlice<'a> {
    pub bar: &'a SacrificeBar,
    pub slice: &'a CutSlice,
}

impl SelectedSlice<'_> {
    pub fn identity(&self) -> String {
        format!("{}-{}", self.bar.id, self.slice.id)
    }
}

pub struct Selection<'a> {
    pub options: Vec<(&'a SacrificeBar, &'a CutOption)>,
    pub slices: Vec<SelectedSlice<'a>>,
}

impl<'a> Selection<'a> {
    pub fn new(picks: impl IntoIterator<Item = (&'a SacrificeBar, &'a CutOption)>) -> Self {
        let mut options = Vec::new();
        let mut slices = Vec::new();
        let mut seen_options = HashSet::new();
        let mut seen_slices = HashSet::new();
        for (bar, option) in picks {
            if seen_options.insert((bar.id, option.id)) {
                options.push((bar, option));
            }
            for slice in bar.slices {
                if slice.cut.is_some()
                    && option.slice_ids.contains(&slice.id)
                    && seen_slices.insert((bar.id, slice.id))
                {
                    slices.push(SelectedSlice { bar, slice });
                }
            }
        }
        Self { options, slices }
    }

    pub fn kg(&self) -> f64 {
        self.slices.iter().map(|selected| selected.slice.kg).sum()
    }

    pub fn erased(&self) -> String {
        self.slices
            .iter()
            .map(SelectedSlice::identity)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::super::reference_data::{HABIT_BARS, SACRIFICE_BARS, cuttable_kg};
    use super::*;

    #[test]
    fn overlapping_climate_cuts_erase_ac_once() {
        let climate = &SACRIFICE_BARS[0];
        let options: Vec<_> = climate
            .options
            .iter()
            .filter(|option| ["no-climate-control", "sweat"].contains(&option.id))
            .map(|option| (climate, option))
            .collect();
        let selected = Selection::new(options.iter().copied());
        assert_eq!(selected.kg(), 3260.0);
        assert_eq!(selected.slices.len(), 4);
        assert_eq!(
            selected
                .erased()
                .split_whitespace()
                .filter(|id| *id == "climate-ac-all")
                .count(),
            1
        );
        assert_eq!(
            Selection::new(options.into_iter().rev()).kg(),
            selected.kg()
        );
    }

    #[test]
    fn duplicate_and_nested_cuts_never_exceed_the_cuttable_baseline() {
        let bars: Vec<_> = SACRIFICE_BARS.iter().chain(HABIT_BARS).collect();
        let picks: Vec<_> = bars
            .iter()
            .flat_map(|bar| bar.options.iter().map(move |option| (*bar, option)))
            .collect();
        let selected = Selection::new(picks.iter().copied().chain(picks.iter().copied()));
        let baseline: f64 = bars.iter().map(|bar| cuttable_kg(bar)).sum();
        assert!((selected.kg() - baseline).abs() < 1e-9);
        assert_eq!(selected.options.len(), picks.len());
        assert_eq!(
            selected.slices.len(),
            bars.iter()
                .flat_map(|bar| bar.slices)
                .filter(|slice| slice.cut.is_some())
                .count()
        );
        assert_eq!(Selection::new([]).kg(), 0.0);
    }
}
