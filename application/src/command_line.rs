use crate::gettext;
use clap::{Arg, Command};
use std::path::PathBuf;

pub struct CommandLineArguments {
    pub input_file: Option<PathBuf>,
    pub disable_gl: bool,
}

pub fn command_line() -> CommandLineArguments {
    let about_msg =
        gettext("Build a table of contents from a media file\nor split a media file into chapters");
    let help_msg = gettext("Display this message");
    let version_msg = gettext("Print version information");

    const DISABLE_GL_ARG: &str = "DISABLE_GL";
    let media = gettext("MEDIA");

    let mut cmd = Command::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(about_msg)
        .arg(
            Arg::new(DISABLE_GL_ARG)
                .short('d')
                .long("disable-gl")
                .help(gettext("Disable video rendering hardware acceleration")),
        )
        .arg(
            Arg::new(media.clone())
                .help(gettext("Path to the input media file"))
                .last(false),
        );
    cmd.build();

    let matches = cmd
        .mut_arg("help", |arg| arg.help(help_msg))
        .mut_arg("version", |arg| arg.help(version_msg))
        .get_matches();

    CommandLineArguments {
        input_file: matches.get_one(media.as_str()).cloned(),
        disable_gl: matches.contains_id(DISABLE_GL_ARG),
    }
}
