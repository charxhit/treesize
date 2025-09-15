use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;

pub fn fuzzy_score(needle: &str, hay: &str) -> Option<i64> {
    let m = SkimMatcherV2::default();
    m.fuzzy_match(hay, needle)
}
