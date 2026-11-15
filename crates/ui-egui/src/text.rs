//! The interface's strings (§4.9).
//!
//! Every user-visible string goes through `craft-i18n`, so WP14 only has to add rows to the
//! translation resources. WP7 builds the catalogue from an empty table: `I18n::t` then returns the
//! English source text unchanged, and the strings stay in one place.

use craft_i18n::I18n;

/// The app's strings, in the app's language.
pub struct Text {
    i18n: I18n,
    lang: craft_i18n::Lang,
}

impl Text {
    /// The strings of the system's first preferred language.
    pub fn new() -> Text {
        let (i18n, _problems) = I18n::new(&[], &[]);
        let lang = i18n.system_lang();
        Text { i18n, lang }
    }

    /// The string in the app's language; the source text when there is no translation yet.
    pub fn t(&self, source: &str) -> String {
        self.i18n.t(self.lang, source).to_owned()
    }

    /// The string with a disambiguating context, for two strings with the same source text.
    pub fn tc(&self, context: &str, source: &str) -> String {
        self.i18n.tc(self.lang, context, source).to_owned()
    }

    /// The wording of a count: "{n} page" or "{n} pages", in the app's language.
    pub fn trn(&self, n: u64, one: &str, other: &str) -> String {
        self.i18n.trn(self.lang, n, one, other)
    }

    /// The language the strings are in.
    pub fn lang(&self) -> craft_i18n::Lang {
        self.lang
    }
}

impl Default for Text {
    fn default() -> Text {
        Text::new()
    }
}

impl std::fmt::Debug for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Text").field("lang", &self.i18n.code(self.lang)).finish()
    }
}
