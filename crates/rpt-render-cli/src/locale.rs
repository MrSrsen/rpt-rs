//! Resolve the effective render locale for the CLI.
//!
//! Precedence (per the SDK grounding): an explicit `--locale` overrides the host OS
//! locale, which overrides the `en-US` fallback. This mirrors the native engine, which reads the
//! host locale once at process start and uses it to resolve "System Default" date/number formats;
//! there is no stored per-report locale to arbitrate.
//!
//! NOTE: this resolves and *reports* the locale, but `rpt-format-value` is still en-US only, so a
//! non-en-US locale does not yet change the formatted output — the CLI warns when that gap bites.

/// Where the resolved locale came from — surfaced in the log so the user can see why.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// An explicit `--locale` flag.
    Flag,
    /// Detected from the host environment (`LC_ALL` / `LC_NUMERIC` / `LANG`).
    Host,
    /// The `en-US` fallback (no flag, no usable host locale).
    Default,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Flag => "--locale flag",
            Source::Host => "host environment",
            Source::Default => "default",
        }
    }
}

/// Resolve the effective locale tag and its source. `explicit` is the `--locale` value if given.
pub fn resolve(explicit: Option<&str>) -> (String, Source) {
    if let Some(l) = explicit {
        return (normalize_tag(l), Source::Flag);
    }
    match host_locale(|k: &str| std::env::var(k)) {
        Some(l) => (l, Source::Host),
        None => ("en-US".to_string(), Source::Default),
    }
}

/// Detect the host locale from the POSIX environment, honouring the precedence
/// `LC_ALL` > `LC_NUMERIC` > `LANG`. `C`/`POSIX` (and unset) are treated as "no locale". `get`
/// is injected (not a global `getenv`) so this is deterministically unit-testable.
fn host_locale(get: impl Fn(&str) -> Result<String, std::env::VarError>) -> Option<String> {
    for key in ["LC_ALL", "LC_NUMERIC", "LANG"] {
        if let Ok(v) = get(key) {
            let v = v.trim();
            if !v.is_empty() && v != "C" && v != "POSIX" {
                return Some(normalize_tag(v));
            }
        }
    }
    None
}

/// Normalize a POSIX/locale string to a BCP-47-ish tag: strip the `.CHARSET`/`@modifier` suffix and
/// turn `ll_CC` into `ll-CC` (`en_US.UTF-8` → `en-US`, `de_DE@euro` → `de-DE`).
fn normalize_tag(s: &str) -> String {
    let base = s.split(['.', '@']).next().unwrap_or(s).trim();
    base.replace('_', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::VarError;

    /// Build a `get` closure over a fixed (key, value) table.
    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Result<String, VarError> + 'a {
        move |k: &str| {
            pairs
                .iter()
                .find(|(pk, _)| *pk == k)
                .map(|(_, v)| v.to_string())
                .ok_or(VarError::NotPresent)
        }
    }

    #[test]
    fn lc_all_wins_over_lang() {
        assert_eq!(
            host_locale(env(&[("LC_ALL", "de_DE.UTF-8"), ("LANG", "en_US.UTF-8")])),
            Some("de-DE".to_string())
        );
    }

    #[test]
    fn lc_numeric_beats_lang_when_no_lc_all() {
        assert_eq!(
            host_locale(env(&[
                ("LC_NUMERIC", "fr_FR.UTF-8"),
                ("LANG", "en_US.UTF-8")
            ])),
            Some("fr-FR".to_string())
        );
    }

    #[test]
    fn c_and_posix_are_ignored() {
        assert_eq!(
            host_locale(env(&[("LC_ALL", "C"), ("LANG", "POSIX")])),
            None
        );
        assert_eq!(host_locale(env(&[])), None);
    }

    #[test]
    fn explicit_flag_overrides_host() {
        // resolve() reads the real env for the host path, but an explicit flag never consults it.
        let (tag, src) = resolve(Some("en_GB.UTF-8"));
        assert_eq!(tag, "en-GB");
        assert_eq!(src, Source::Flag);
    }

    #[test]
    fn modifier_suffix_stripped() {
        assert_eq!(normalize_tag("de_DE@euro"), "de-DE");
        assert_eq!(normalize_tag("en_US"), "en-US");
    }
}
