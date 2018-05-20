use directories::ProjectDirs;
use gettextrs::{TextDomain, TextDomainError};

use super::{TLD, SLD};

pub fn init_locale() {
    // Search translations under `target` first
    // in order to reflect latest changes during development
    let text_domain = TextDomain::new(env!("CARGO_PKG_NAME")).prepend("target");

    // Add user's data dir in the search path
    let project_dirs = ProjectDirs::from(TLD, SLD, env!("CARGO_PKG_NAME"));
    let app_data_dir = project_dirs.data_dir();

    // FIXME: figure out macOS conventions
    #[cfg(all(target_family = "unix", not(target_os = "macos")))]
    let text_domain = match app_data_dir.parent() {
        Some(data_dir) => text_domain.prepend(data_dir),
        None => text_domain,
    };

    #[cfg(target_os = "windows")]
    let text_domain = text_domain.prepend(app_data_dir);

    match text_domain.init() {
        Ok(locale) => info!("Translation found, `setlocale` returned {:?}", locale),
        Err(TextDomainError::TranslationNotFound(lang)) => {
            warn!("Translation not found for language {}", lang)
        }
        Err(TextDomainError::InvalidLocale(locale)) => error!("Invalid locale {}", locale),
    }
}