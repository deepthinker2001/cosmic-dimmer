//! Fluent localization, embedded at build time (the COSMIC applet convention).

use std::sync::LazyLock;

use i18n_embed::{
    DefaultLocalizer, LanguageLoader, Localizer,
    fluent::{FluentLanguageLoader, fluent_language_loader},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

pub static LANGUAGE_LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader: FluentLanguageLoader = fluent_language_loader!();
    // The fallback bundle is embedded; failing to parse it is a build defect.
    loader
        .load_fallback_language(&Localizations)
        .expect("embedded fallback language must load");
    loader
});

/// Look up a localized message by ID, checked against the fallback at compile time.
#[macro_export]
macro_rules! fl {
    ($message_id:literal) => {{
        i18n_embed_fl::fl!($crate::localize::LANGUAGE_LOADER, $message_id)
    }};
    ($message_id:literal, $($args:expr),*) => {{
        i18n_embed_fl::fl!($crate::localize::LANGUAGE_LOADER, $message_id, $($args), *)
    }};
}

/// `text` without the bidi isolation marks Fluent puts round each argument,
/// for output to a terminal, which shows them as stray characters.
#[must_use]
pub fn plain(text: &str) -> String {
    text.replace(['\u{2068}', '\u{2069}'], "")
}

/// Select the user's desktop languages, keeping the fallback if none match.
pub fn localize() {
    let localizer = DefaultLocalizer::new(&*LANGUAGE_LOADER, &Localizations);
    let requested = i18n_embed::DesktopLanguageRequester::requested_languages();
    if let Err(error) = localizer.select(&requested) {
        tracing::warn!(%error, "falling back to built-in strings");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    /// Each message's ID and the variables it uses, from Fluent source. A
    /// message starts at a line `id = ...` in column 0; its continuation
    /// lines are indented.
    fn messages(source: &str) -> BTreeMap<String, BTreeSet<String>> {
        let mut messages = BTreeMap::new();
        let mut current: Option<String> = None;
        for line in source.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            if !line.starts_with(' ')
                && let Some((id, _)) = line.split_once(" =")
            {
                current = Some(id.trim().to_owned());
                messages.insert(id.trim().to_owned(), BTreeSet::new());
            }
            if let Some(id) = &current {
                let variables = messages.get_mut(id).expect("inserted above");
                for part in line.split('$').skip(1) {
                    let name: String = part
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                        .collect();
                    variables.insert(name);
                }
            }
        }
        messages
    }

    fn source(language: &str) -> String {
        let file = Localizations::get(&format!("{language}/cosmic-dimmer.ftl"))
            .expect("the language's file is embedded");
        String::from_utf8(file.data.into_owned()).expect("UTF-8")
    }

    #[test]
    fn every_translation_has_every_message_and_its_variables() {
        let english = messages(&source("en"));
        assert!(english.len() >= 40, "parsed the English file");
        let languages: BTreeSet<String> = Localizations::iter()
            .filter_map(|path| path.split_once('/').map(|(lang, _)| lang.to_owned()))
            .collect();
        for language in languages.iter().filter(|lang| *lang != "en") {
            let translated = messages(&source(language));
            assert_eq!(
                translated.keys().collect::<Vec<_>>(),
                english.keys().collect::<Vec<_>>(),
                "{language}: the same message IDs as English"
            );
            for (id, variables) in &english {
                assert_eq!(
                    &translated[id], variables,
                    "{language}: {id} uses the same variables"
                );
            }
        }
    }

    #[test]
    fn every_translation_loads() {
        for path in Localizations::iter() {
            let Some((language, _)) = path.split_once('/') else {
                continue;
            };
            let loader: FluentLanguageLoader = fluent_language_loader!();
            let id: i18n_embed::unic_langid::LanguageIdentifier =
                language.parse().expect("folder names are language IDs");
            loader
                .load_languages(&Localizations, &[id])
                .unwrap_or_else(|error| panic!("{language} does not load: {error}"));
        }
    }
}
