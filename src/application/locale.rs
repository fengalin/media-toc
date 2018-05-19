use gettextrs::{TextDomain, TextDomainError};

pub fn init_locale() {
    match TextDomain::new("media-toc").prepend("target").init() {
        Ok(locale) => info!("Translation found, `setlocale` returned {:?}", locale),
        Err(TextDomainError::TranslationNotFound(lang)) => {
            warn!("Translation not found for language {}", lang)
        }
        Err(TextDomainError::InvalidLocale(locale)) => error!("Invalid locale {}", locale),
    }
}