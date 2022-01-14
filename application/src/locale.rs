use directories::ProjectDirs;
use gettextrs::{TextDomain, TextDomainError};
use log::{error, info, warn};

use super::{APP_NAME, SLD, TLD};

pub fn init_locale() {
    // Search translations under `target` first
    // in order to reflect latest changes during development
    let text_domain = TextDomain::new(&*APP_NAME)
        .codeset("UTF-8")
        .prepend("target");

    // Add user's data dir in the search path
    let project_dirs = ProjectDirs::from(TLD, SLD, &APP_NAME)
        .expect("Couldn't find project dirs for this platform");
    let _app_data_dir = project_dirs.data_dir();

    // FIXME: figure out macOS conventions
    #[cfg(all(target_family = "unix", not(target_os = "macos")))]
    let text_domain = match _app_data_dir.parent() {
        Some(data_dir) => text_domain.prepend(data_dir),
        None => text_domain,
    };

    #[cfg(target_os = "windows")]
    let text_domain = text_domain.prepend(_app_data_dir);

    match text_domain.init() {
        Ok(locale) => info!("Translation found, `setlocale` returned {:?}", locale),
        Err(TextDomainError::TranslationNotFound(lang)) => {
            warn!("Translation not found for language {}", lang)
        }
        Err(TextDomainError::InvalidLocale(locale)) => error!("Invalid locale {}", locale),
        Err(err) => error!("Couldn't set locale {}", err),
    }
}
