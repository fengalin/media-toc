use directories::ProjectDirs;
use gettextrs::gettext;
use lazy_static::lazy_static;
use log::{debug, error};
use ron;
use serde::{Deserialize, Serialize};

use std::{
    fs::{create_dir_all, File},
    io::Write,
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::RwLock,
};

use super::{APP_NAME, SLD, TLD};

const CONFIG_FILENAME: &str = "config.ron";

lazy_static! {
    pub static ref CONFIG: RwLock<GlobalConfig> = RwLock::new(GlobalConfig::new());
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct UI {
    pub width: i32,
    pub height: i32,
    pub paned_pos: i32,
    pub is_chapters_list_hidden: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Media {
    pub is_gl_disabled: bool,
    pub last_path: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Config {
    pub ui: UI,
    pub media: Media,
}

pub struct GlobalConfig {
    path: PathBuf,
    last: Config,
    current: Config,
}

impl GlobalConfig {
    fn new() -> GlobalConfig {
        let project_dirs = ProjectDirs::from(TLD, SLD, &APP_NAME)
            .expect("Couldn't find project dirs for this platform");
        let config_dir = project_dirs.config_dir();
        create_dir_all(&config_dir).unwrap();
        let path = config_dir.join(CONFIG_FILENAME);

        let last = match File::open(&path) {
            Ok(config_file) => {
                let config: Result<Config, ron::de::Error> = ron::de::from_reader(config_file);
                match config {
                    Ok(config) => {
                        debug!("read config: {:?}", config);
                        config
                    }
                    Err(err) => {
                        error!(
                            "{}",
                            &gettext("couldn't load configuration: {}").replacen(
                                "{}",
                                &format!("{:?}", err),
                                1
                            ),
                        );
                        Config::default()
                    }
                }
            }
            Err(_) => Config::default(),
        };

        GlobalConfig {
            path,
            current: last.clone(),
            last,
        }
    }

    pub fn save(&mut self) {
        if self.last == self.current {
            // unchanged => don't save
            return;
        }

        match File::create(&self.path) {
            Ok(mut config_file) => {
                match ron::ser::to_string_pretty(&self.current, ron::ser::PrettyConfig::default()) {
                    Ok(config_str) => match config_file.write_all(config_str.as_bytes()) {
                        Ok(()) => {
                            self.last = self.current.clone();
                            debug!("saved config: {:?}", self.current);
                        }
                        Err(err) => {
                            error!(
                                "{}",
                                &gettext("couldn't write configuration: {}").replacen(
                                    "{}",
                                    &format!("{:?}", err),
                                    1
                                ),
                            );
                        }
                    },
                    Err(err) => {
                        error!(
                            "{}",
                            &gettext("couldn't serialize configuration: {}").replacen(
                                "{}",
                                &format!("{:?}", err),
                                1
                            ),
                        );
                    }
                }
            }
            Err(err) => {
                error!(
                    "{}",
                    &gettext("couldn't create configuration file: {}").replacen(
                        "{}",
                        &format!("{:?}", err),
                        1
                    ),
                );
            }
        }
    }
}

impl Deref for GlobalConfig {
    type Target = Config;

    fn deref(&self) -> &Self::Target {
        &self.current
    }
}

impl DerefMut for GlobalConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.current
    }
}
