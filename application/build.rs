#[cfg(target_family = "unix")]
use directories::{BaseDirs, ProjectDirs};

use lazy_static::lazy_static;

use std::fs::{create_dir_all, File};
use std::io::{ErrorKind, Read};
use std::path::PathBuf;
use std::process::Command;

#[cfg(target_family = "unix")]
use std::fs::read_dir;
#[cfg(target_family = "unix")]
use std::io::Write;

lazy_static! {
    // Remove "-application" from `CARGO_PKG_NAME`
    pub static ref APP_NAME: String = env!("CARGO_PKG_NAME")
        .rsplitn(2, '-')
        .last()
        .unwrap()
        .to_string();
}

fn po_path() -> PathBuf {
    PathBuf::from("..").join("po")
}

fn res_path() -> PathBuf {
    PathBuf::from("..").join("res")
}

fn target_path() -> PathBuf {
    PathBuf::from("..").join("target")
}

fn generate_resources() {
    let output_path = target_path().join("resources");
    create_dir_all(&output_path).unwrap();

    // Icons
    let input_path = res_path().join("icons").join("hicolor");

    let mut compile_res = Command::new("glib-compile-resources");
    compile_res
        .arg("--generate")
        .arg(format!("--sourcedir={}", input_path.to_str().unwrap()))
        .arg(format!(
            "--target={}",
            output_path.join("icons.gresource").to_str().unwrap(),
        ))
        .arg(input_path.join("icons.gresource.xml").to_str().unwrap());

    match compile_res.status() {
        Ok(status) => {
            if !status.success() {
                panic!(
                    "Failed to generate resources file for icons\n{:?}",
                    compile_res,
                );
            }
        }
        Err(ref error) => match error.kind() {
            ErrorKind::NotFound => panic!(
                "Can't generate icon resources: command `glib-compile-resources` not available"
            ),
            _ => panic!("Error invoking `compile_res`: {}", error),
        },
    }

    // UI
    let input_path = res_path().join("ui");

    let mut compile_res = Command::new("glib-compile-resources");
    compile_res
        .arg("--generate")
        .arg(format!("--sourcedir={}", input_path.to_str().unwrap()))
        .arg(format!(
            "--target={}",
            output_path.join("ui.gresource").to_str().unwrap(),
        ))
        .arg(input_path.join("ui.gresource.xml").to_str().unwrap());

    match compile_res.status() {
        Ok(status) => {
            if !status.success() {
                panic!(
                    "Failed to generate resources file for the UI\n{:?}",
                    compile_res,
                );
            }
        }
        Err(ref error) => match error.kind() {
            ErrorKind::NotFound => panic!(
                "Can't generate UI resources: command `glib-compile-resources` not available"
            ),
            _ => panic!("Error invoking `compile_res`: {}", error),
        },
    }
}

fn generate_translations() {
    if let Ok(mut linguas_file) = File::open(&po_path().join("LINGUAS")) {
        let mut linguas = String::new();
        linguas_file
            .read_to_string(&mut linguas)
            .expect("Couldn't read po/LINGUAS as string");

        for lingua in linguas.lines() {
            let mo_path = target_path()
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
                .arg(format!("--directory={}", po_path().to_str().unwrap()))
                .arg(format!("{}.po", lingua));

            match msgfmt.status() {
                Ok(status) => {
                    if !status.success() {
                        panic!(
                            "Failed to generate mo file for lingua {}\n{:?}",
                            lingua, msgfmt,
                        );
                    }
                }
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
    let base_dirs = BaseDirs::new().unwrap();
    // Note: `base_dirs.executable_dir()` is `None` on macOS
    if let Some(exe_dir) = base_dirs.executable_dir() {
        let project_dirs = ProjectDirs::from("org", "fengalin", &APP_NAME).unwrap();
        let app_data_dir = project_dirs.data_dir();
        let data_dir = app_data_dir.parent().unwrap();

        match File::create(&target_path().join("install")) {
            Ok(mut install_file) => {
                install_file
                    .write_all(format!("# User install script for {}\n", *APP_NAME).as_bytes())
                    .unwrap();

                install_file.write_all(b"\n# Install executable\n").unwrap();
                install_file
                    .write_all(format!("mkdir -p {:?}\n", exe_dir).as_bytes())
                    .unwrap();
                install_file
                    .write_all(
                        format!(
                            "cp {:?} {:?}\n",
                            target_path()
                                .canonicalize()
                                .unwrap()
                                .join("release")
                                .join(&*APP_NAME),
                            exe_dir.join(&*APP_NAME),
                        )
                        .as_bytes(),
                    )
                    .unwrap();

                install_file.write_all(b"\n# Install icons\n").unwrap();
                let icon_target_dir = data_dir.join("icons").join("hicolor");
                let entry_iter = read_dir(res_path().join("icons").join("hicolor")).unwrap();
                for entry in entry_iter {
                    let entry = entry.unwrap();
                    let entry_path = entry.path();
                    if entry_path.is_dir() {
                        let target_dir = icon_target_dir.join(&entry.file_name());
                        install_file
                            .write_all(format!("mkdir -p {:?}\n", target_dir).as_bytes())
                            .unwrap();

                        install_file
                            .write_all(
                                format!(
                                    "cp -r {:?} {:?}\n",
                                    entry_path.canonicalize().unwrap().join("apps"),
                                    target_dir,
                                )
                                .as_bytes(),
                            )
                            .unwrap();
                    }
                }

                install_file
                    .write_all(b"\n# Install translations\n")
                    .unwrap();
                install_file
                    .write_all(format!("mkdir -p {:?}\n", data_dir).as_bytes())
                    .unwrap();
                install_file
                    .write_all(
                        format!(
                            "cp -r {:?} {:?}\n",
                            target_path().join("locale").canonicalize().unwrap(),
                            data_dir,
                        )
                        .as_bytes(),
                    )
                    .unwrap();

                install_file
                    .write_all(b"\n# Install desktop file\n")
                    .unwrap();
                let desktop_target_dir = data_dir.join("applications");
                install_file
                    .write_all(format!("mkdir -p {:?}\n", desktop_target_dir).as_bytes())
                    .unwrap();
                install_file
                    .write_all(
                        format!(
                            "cp {:?} {:?}\n",
                            res_path()
                                .join(&format!("org.fengalin.{}.desktop", *APP_NAME))
                                .canonicalize()
                                .unwrap(),
                            desktop_target_dir,
                        )
                        .as_bytes(),
                    )
                    .unwrap();
            }
            Err(err) => panic!("Couldn't create file `target/install`: {:?}", err),
        }
    }
}

// FIXME: figure out macOS conventions for icons & translations
#[cfg(target_family = "unix")]
fn generate_uninstall_script() {
    let base_dirs = BaseDirs::new().unwrap();
    // Note: `base_dirs.executable_dir()` is `None` on macOS
    if let Some(exe_dir) = base_dirs.executable_dir() {
        let project_dirs = ProjectDirs::from("org", "fengalin", &APP_NAME).unwrap();
        let app_data_dir = project_dirs.data_dir();
        let data_dir = app_data_dir.parent().unwrap();

        match File::create(&target_path().join("uninstall")) {
            Ok(mut install_file) => {
                install_file
                    .write_all(format!("# User uninstall script for {}\n", *APP_NAME).as_bytes())
                    .unwrap();

                install_file
                    .write_all(b"\n# Uninstall executable\n")
                    .unwrap();
                install_file
                    .write_all(format!("rm {:?}\n", exe_dir.join(&*APP_NAME)).as_bytes())
                    .unwrap();
                install_file
                    .write_all(format!("rmdir -p {:?}\n", exe_dir).as_bytes())
                    .unwrap();

                install_file.write_all(b"\n# Uninstall icons\n").unwrap();
                let icon_target_dir = data_dir.join("icons").join("hicolor");
                let entry_iter = read_dir(res_path().join("icons").join("hicolor")).unwrap();
                for entry in entry_iter {
                    let entry = entry.unwrap();
                    let entry_path = entry.path();
                    if entry_path.is_dir() {
                        let icon_target_dir = icon_target_dir
                            .join(entry_path.file_name().unwrap())
                            .join("apps");
                        let icons_subdir_iter = read_dir(entry_path.join("apps")).unwrap();
                        for icon_subdir_entry in icons_subdir_iter {
                            let icon_subdir_entry = icon_subdir_entry.unwrap();
                            install_file
                                .write_all(
                                    format!(
                                        "rm {:?}\n",
                                        icon_target_dir.join(icon_subdir_entry.file_name()),
                                    )
                                    .as_bytes(),
                                )
                                .unwrap();
                        }

                        install_file
                            .write_all(format!("rmdir -p {:?}\n", icon_target_dir).as_bytes())
                            .unwrap();
                    }
                }

                if let Ok(mut linguas_file) = File::open(&po_path().join("LINGUAS")) {
                    let mut linguas = String::new();
                    linguas_file
                        .read_to_string(&mut linguas)
                        .expect("Couldn't read po/LINGUAS as string");

                    install_file
                        .write_all(b"\n# Uninstall translations\n")
                        .unwrap();
                    let locale_base_dir = data_dir.join("locale");
                    for lingua in linguas.lines() {
                        let lingua_dir = locale_base_dir.join(lingua).join("LC_MESSAGES");
                        install_file
                            .write_all(
                                format!(
                                    "rm {:?}\n",
                                    lingua_dir.join(&format!("{}.mo", *APP_NAME)),
                                )
                                .as_bytes(),
                            )
                            .unwrap();

                        install_file
                            .write_all(format!("rmdir -p {:?}\n", lingua_dir).as_bytes())
                            .unwrap();
                    }
                }

                install_file
                    .write_all(b"\n# Uninstall desktop file\n")
                    .unwrap();
                let desktop_target_dir = data_dir.join("applications");
                install_file
                    .write_all(
                        format!(
                            "rm {:?}\n",
                            desktop_target_dir.join(&format!("org.fengalin.{}.desktop", *APP_NAME)),
                        )
                        .as_bytes(),
                    )
                    .unwrap();
                install_file
                    .write_all(format!("rmdir -p {:?}\n", desktop_target_dir).as_bytes())
                    .unwrap();
            }
            Err(err) => panic!("Couldn't create file `target/uninstall`: {:?}", err),
        }
    }
}

fn main() {
    generate_resources();
    generate_translations();

    #[cfg(target_family = "unix")]
    generate_install_script();

    #[cfg(target_family = "unix")]
    generate_uninstall_script();
}
