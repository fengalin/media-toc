use directories::ProjectDirs;
use gettextrs::gettext;
use ron;

use std::fs::{create_dir_all, File};
use std::io::Write;
use std::path::PathBuf;

use super::{TLD, SLD};

const CONFIG_FILENAME: &str = "config.ron";

lazy_static! {
    pub static ref CONFIG_PATH: PathBuf = {
        let project_dirs = ProjectDirs::from(TLD, SLD, env!("CARGO_PKG_NAME"));
        let config_dir = project_dirs.config_dir();
        create_dir_all(&config_dir).unwrap();
        config_dir.join(CONFIG_FILENAME).to_owned()
    };
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct UI {
    pub width: i32,
    pub height: i32,
    pub is_chapters_list_hidden: bool,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Media {
    pub is_gl_disable: bool,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Config {
    pub ui: UI,
    pub media: Media,
}

impl Config {
    pub fn get() -> Config {
        match File::open(&*CONFIG_PATH) {
            Ok(config_file) => {
                let config: Result<Config, ron::de::Error> = ron::de::from_reader(config_file);
                match config {
                    Ok(config) => {
                        debug!("read config: {:?}", config);
                        config
                    }
                    Err(err) => {
                        error!("{}",
                            &gettext("couldn't load configuration: {}")
                                .replacen("{}", &format!("{:?}", err), 1),
                        );
                        Config::default()
                    }
                }
            }
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) {
        match File::create(&*CONFIG_PATH) {
            Ok(mut config_file) => {
                let config_se = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default());
                match config_se {
                    Ok(config_str) => match config_file.write_all(config_str.as_bytes()) {
                        Ok(()) => debug!("saved config: {:?}", self),
                        Err(err) => {
                            error!("{}",
                                &gettext("couldn't write configuration: {}")
                                    .replacen("{}", &format!("{:?}", err), 1),
                            );
                        }
                    }
                    Err(err) => {
                        error!("{}",
                            &gettext("couldn't serialize configuration: {}")
                                .replacen("{}", &format!("{:?}", err), 1),
                        );
                    }
                }
            }
            Err(err) => {
                error!("{}",
                    &gettext("couldn't create configuration file: {}")
                        .replacen("{}", &format!("{:?}", err), 1),
                );
            }
        }
    }
}