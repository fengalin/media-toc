use clap::{App, Arg};
use gettextrs::gettext;

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
    let input_arg = gettext("MEDIA");

    let matches = App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(about_msg.as_str())
        .help_message(help_msg.as_str())
        .version_message(version_msg.as_str())
        .arg(
            Arg::with_name(DISABLE_GL_ARG)
                .short("d")
                .long("disable-gl")
                .help(&gettext("Disable video rendering hardware acceleration")),
        )
        .arg(
            Arg::with_name(input_arg.as_str())
                .help(&gettext("Path to the input media file"))
                .last(false),
        )
        .get_matches();

    CommandLineArguments {
        input_file: matches
            .value_of(input_arg.as_str())
            .map(|input_file| input_file.into()),
        disable_gl: matches.is_present(DISABLE_GL_ARG),
    }
}
