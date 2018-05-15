use std::fs::{create_dir_all, File};
use std::io::{ErrorKind, Read};
use std::path::PathBuf;
use std::process::Command;

fn generate_resources() {
    let target_path = PathBuf::from("target")
        .join("resources");
    create_dir_all(&target_path).unwrap();

    let input_path = PathBuf::from("assets").join("icons").join("hicolor");

    let mut compile_res = Command::new("glib-compile-resources");
    compile_res
        .arg("--generate")
        .arg(format!("--sourcedir={}", input_path.to_str().unwrap()))
        .arg(format!("--target={}",
            target_path.join("icons.gresource").to_str().unwrap()
        ))
        .arg(input_path.join("icons.gresource.xml").to_str().unwrap());

    match compile_res.status() {
        Ok(status) => if !status.success() {
            panic!(format!(
                "Failed to generate resources file for icons\n{:?}",
                compile_res,
            ));
        },
        Err(ref error) => match error.kind() {
            ErrorKind::NotFound => {
                eprintln!("Can't generate translations: command `compile_res` not available");
                return;
            }
            _ => panic!("Error invoking `compile_res`: {}", error),
        },
    }
}

fn generate_translations() {
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
            msgfmt
                .arg(format!(
                    "--output-file={}",
                    mo_path.join("media-toc.mo").to_str().unwrap()
                ))
                .arg("--directory=po")
                .arg(format!("{}.po", lingua));

            match msgfmt.status() {
                Ok(status) => if !status.success() {
                    panic!(format!(
                        "Failed to generate mo file for lingua {}\n{:?}",
                        lingua, msgfmt,
                    ));
                },
                Err(ref error) => match error.kind() {
                    ErrorKind::NotFound => {
                        eprintln!("Can't generate translations: command `msgfmt` not available");
                        return;
                    }
                    _ => panic!("Error invoking `msgfmt`: {}", error),
                },
            }
        }
    }
}

fn main() {
    generate_resources();
    generate_translations();
}