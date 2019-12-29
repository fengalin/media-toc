use lazy_static::lazy_static;

pub const TLD: &str = "org";
pub const SLD: &str = "fengalin";

lazy_static! {
    // Remove "-application" from `CARGO_PKG_NAME`
    pub static ref APP_NAME: String = env!("CARGO_PKG_NAME")
        .rsplitn(2, '-')
        .last()
        .unwrap()
        .to_string();
}

lazy_static! {
    pub static ref APP_ID: String = format!("{}.{}.{}", TLD, SLD, *APP_NAME);
}

lazy_static! {
    pub static ref APP_PATH: String = format!("/{}/{}/{}", TLD, SLD, *APP_NAME);
}

mod command_line;
pub use self::command_line::{get_command_line, CommandLineArguments};

mod configuration;
pub use self::configuration::CONFIG;

mod locale;
pub use self::locale::init_locale;
