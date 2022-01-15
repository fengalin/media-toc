use once_cell::sync::Lazy;

pub const TLD: &str = "org";
pub const SLD: &str = "fengalin";

// Remove "-application" from `CARGO_PKG_NAME`
pub static APP_NAME: Lazy<String> = Lazy::new(|| {
    env!("CARGO_PKG_NAME")
        .rsplitn(2, '-')
        .last()
        .unwrap()
        .to_string()
});

pub static APP_ID: Lazy<String> = Lazy::new(|| format!("{}.{}.{}", TLD, SLD, *APP_NAME));
pub static APP_PATH: Lazy<String> = Lazy::new(|| format!("/{}/{}/{}", TLD, SLD, *APP_NAME));

mod command_line;
pub use self::command_line::{command_line, CommandLineArguments};

mod configuration;
pub use self::configuration::CONFIG;

mod locale;
pub use self::locale::init_locale;
