use std::fs::{File, create_dir_all};
use std::io::{ErrorKind, Read};
use std::path::PathBuf;
use std::process::Command;

fn main() {
    if let Ok(mut linguas_file) = File::open(&PathBuf::from("po").join("LINGUAS")) {
        let mut linguas = String::new();
        linguas_file
            .read_to_string(&mut linguas)
            .expect("Couldn't read po/LINGUAS as string");

        for lingua in linguas.lines() {
            let mo_path = PathBuf::from("target")
                .join("locale")
                .join(lingua)
                .join("LC_MESSAGES");
            create_dir_all(&mo_path).unwrap();

            let mut msgfmt = Command::new("msgfmt");
            msgfmt.arg(format!("--output-file={}",
                    mo_path.join("media-toc.mo")
                        .to_str()
                        .unwrap()
                ))
                .arg("--directory=po")
                .arg(format!("{}.po", lingua));

            match msgfmt.status() {
                Ok(status) => if !status.success() {
                    panic!(format!("Failed to generate mo file for lingua {}\n{:?}",
                        lingua,
                        msgfmt,
                    ));
                }
                Err(ref error) => match error.kind() {
                    ErrorKind::NotFound => {
                        eprintln!("Can't generate translations: command `msgfmt` not available");
                        return;
                    }
                    _ => panic!("Error invoking `msgfmt`: {}", error),
                }
            }
        }
    }
}
