//! Parser for TunerStudio ECU-definition INI files (the Speeduino subset).
//!
//! The format is INI-shaped but not INI: values are comma-separated token
//! lists, there is a `#define`/`#if` preprocessor, and many fields may be
//! `{ expression }` blocks evaluated against other constants. Parsing is
//! implemented as a preprocessor pass followed by per-section line parsers
//! producing a typed definition model.

#[cfg(test)]
mod tests {
    /// The real Speeduino INI this crate is developed against. Golden
    /// assertions about its contents live here as the parser grows.
    const FIXTURE: &str = include_str!("../../../fixtures/speeduino202405_dev.ini");

    #[test]
    fn fixture_is_present_and_looks_like_a_ts_ini() {
        assert!(FIXTURE.lines().count() > 5000);
        for section in ["[Constants]", "[OutputChannels]", "[TableEditor]"] {
            assert!(
                FIXTURE.lines().any(|l| l.trim() == section),
                "missing {section}"
            );
        }
    }
}
