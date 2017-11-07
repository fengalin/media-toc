extern crate gtk;
use gtk::{ButtonExt, GtkWindowExt, WidgetExt};

use std::fs::File;
use std::io::Write;

use std::rc::Rc;
use std::cell::RefCell;

use media::Context;

use super::MainController;

pub struct ExportController {
    export_dlg: gtk::Dialog,
    export_btn: gtk::Button,

    pub context: Option<Context>,
}

impl ExportController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let main_window: gtk::ApplicationWindow = builder.get_object("application-window").unwrap();
        let export_dlg: gtk::Dialog = builder.get_object("export-dlg").unwrap();
        export_dlg.set_transient_for(&main_window);

        Rc::new(RefCell::new(ExportController {
            export_dlg: export_dlg,
            export_btn: builder.get_object("export-btn").unwrap(),
            context: None,
        }))
    }

    pub fn register_callbacks(this_rc: &Rc<RefCell<Self>>, _: &Rc<RefCell<MainController>>) {
        let this = this_rc.borrow();

        let this_clone = Rc::clone(this_rc);
        this.export_btn.connect_clicked(move |_| {
            let this = this_clone.borrow();

            // for the moment, only export to mkvmerge text format is supported
            let ref media_path = this.context.as_ref().unwrap().path;

            let mut target_path = media_path.clone();
            target_path.pop();
            target_path.push(&format!("{}.txt",
                media_path.file_stem()
                    .expect("ExportController::export_btn clicked, failed to get file_stem")
                    .to_str()
                    .expect("ExportController::export_btn clicked, failed to get file_stem as str")
            ));

            // TODO: handle file related errors
            let mut output_file = File::create(target_path)
                .expect("ExportController::export_btn clicked couldn't create output file");

            {
                let context = this.context.as_ref()
                    .unwrap();
                let info = context.info.lock()
                    .expect(
                        "ExportController::export_btn clicked, failed to lock media info",
                    );
                for (index, ref chapter) in info.chapters.iter().enumerate() {
                    let prefix = format!("CHAPTER{:02}", index + 1);
                    output_file.write_fmt(
                        format_args!("{}={}\n",
                            prefix,
                            chapter.start.format_with_hours(),
                        ))
                        .expect("ExportController::export_btn clicked, failed to write in file");
                    output_file.write_fmt(format_args!("{}NAME={}\n", prefix, chapter.title()))
                        .expect("ExportController::export_btn clicked, failed to write in file");
                }
            }

            this.export_dlg.hide();
        });
    }

    pub fn open(&mut self, context: Context) {
        self.context = Some(context);
        self.export_dlg.present();
    }
}
