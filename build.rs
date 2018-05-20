extern crate directories;
use directories::{BaseDirs, ProjectDirs};

use std::fs::{create_dir_all, read_dir, File};
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::process::Command;

fn generate_resources() {
    let target_path = PathBuf::from("target")
        .join("resources");
    create_dir_all(&target_path).unwrap();

    // Icons
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
                panic!("Can't generate translations: command `compile_res` not available");
            }
            _ => panic!("Error invoking `compile_res`: {}", error),
        },
    }

    // UI
    let input_path = PathBuf::from("assets").join("ui");

    let mut compile_res = Command::new("glib-compile-resources");
    compile_res
        .arg("--generate")
        .arg(format!("--sourcedir={}", input_path.to_str().unwrap()))
        .arg(format!("--target={}",
            target_path.join("ui.gresource").to_str().unwrap()
        ))
        .arg(input_path.join("ui.gresource.xml").to_str().unwrap());

    match compile_res.status() {
        Ok(status) => if !status.success() {
            panic!(format!(
                "Failed to generate resources file for the UI\n{:?}",
                compile_res,
            ));
        },
        Err(ref error) => match error.kind() {
            ErrorKind::NotFound => {
                panic!("Can't generate translations: command `compile_res` not available");
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

// FIXME: figure out macOS conventions for icons & translations
#[cfg(target_family = "unix")]
fn generate_install_script() {
    let base_dirs = BaseDirs::new();
    // Note: `base_dirs.executable_dir()` is `None` on macOS
    if let Some(exe_dir) = base_dirs.executable_dir() {
        let project_dirs = ProjectDirs::from("org", "fengalin", env!("CARGO_PKG_NAME"));
        let app_data_dir = project_dirs.data_dir();
        let data_dir = app_data_dir.parent().unwrap();

        match File::create(&PathBuf::from("target").join("install")) {
            Ok(mut install_file) => {
                install_file.write_all(format!("# User install script for {}\n",
                    env!("CARGO_PKG_NAME"),
                ).as_bytes()).unwrap();

                install_file.write_all(b"\n# Install executable\n").unwrap();
                let exe_source_path = PathBuf::from("target")
                    .join("release")
                    .join(env!("CARGO_PKG_NAME"));
                install_file.write_all(format!("mkdir -p {}\n",
                    exe_dir.to_str().unwrap(),
                ).as_bytes()).unwrap();
                install_file.write_all(format!("cp {} {}\n",
                    exe_source_path.to_str().unwrap(),
                    exe_dir.join(env!("CARGO_PKG_NAME")).to_str().unwrap(),
                ).as_bytes()).unwrap();

                install_file.write_all(b"\n# Install icons\n").unwrap();
                let icon_target_dir = data_dir.join("icons").join("hicolor");
                let mut entry_iter = read_dir(
                        PathBuf::from("assets")
                            .join("icons")
                            .join("hicolor")
                    ).unwrap();
                for entry in entry_iter {
                    let entry = entry.unwrap();
                    let entry_path = entry.path();
                    if entry_path.is_dir() {
                        let target_dir = icon_target_dir.join(&entry.file_name());
                        install_file.write_all(format!("mkdir -p {}\n",
                            target_dir.to_str().unwrap()
                        ).as_bytes()).unwrap();

                        install_file.write_all(format!("cp -r {:?}/* {:?}\n",
                            entry_path.to_str().unwrap(),
                            target_dir,
                        ).as_bytes()).unwrap();
                    }
                }

                install_file.write_all(b"\n# Install translations\n").unwrap();
                install_file.write_all(format!("mkdir -p {}\n",
                    data_dir.to_str().unwrap(),
                ).as_bytes()).unwrap();
                install_file.write_all(format!("cp -r {} {}\n",
                    PathBuf::from("target").join("locale").to_str().unwrap(),
                    data_dir.to_str().unwrap(),
                ).as_bytes()).unwrap();

                install_file.write_all(b"\n# Install desktop file\n").unwrap();
                let desktop_target_dir = data_dir.join("applications");
                install_file.write_all(format!("mkdir -p {}\n",
                    desktop_target_dir.to_str().unwrap(),
                ).as_bytes()).unwrap();
                install_file.write_all(format!("cp {} {}\n",
                    PathBuf::from("assets")
                            .join(&format!("org.fengalin.{}.desktop", env!("CARGO_PKG_NAME")))
                            .to_str()
                            .unwrap(),
                    desktop_target_dir.to_str().unwrap(),
                ).as_bytes()).unwrap();
            }
            Err(err) => panic!("Couldn't create file `target/install`: {:?}", err),
        }
    }
}

fn main() {
    generate_resources();
    generate_translations();

    #[cfg(target_family = "unix")]
    generate_install_script();
}