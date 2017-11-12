extern crate gtk;
use gtk::prelude::*;

use std::fs::File;

use std::rc::Rc;
use std::cell::RefCell;

use media::Context;

use toc;

use super::MainController;

pub struct ExportController {
    export_dlg: gtk::Dialog,
    export_btn: gtk::Button,

    mkvmerge_txt_rdbtn: gtk::RadioButton,
    cue_rdbtn: gtk::RadioButton,

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

            mkvmerge_txt_rdbtn: builder.get_object("mkvmerge_txt-rdbtn").unwrap(),
            cue_rdbtn: builder.get_object("cue-rdbtn").unwrap(),

            context: None,
        }))
    }

    pub fn register_callbacks(
        this_rc: &Rc<RefCell<Self>>,
        main_ctrl: &Rc<RefCell<MainController>>
    ) {
        let this = this_rc.borrow();

        let this_clone = Rc::clone(this_rc);
        let main_ctrl_clone = Rc::clone(main_ctrl);
        this.export_dlg.connect_delete_event(move |dlg, _| {
            main_ctrl_clone.borrow_mut().restore_context(
                this_clone.borrow_mut().context.take().unwrap()
            );
            dlg.hide_on_delete();
            Inhibit(true)
        });

        let this_clone = Rc::clone(this_rc);
        let main_ctrl_clone = Rc::clone(main_ctrl);
        this.export_btn.connect_clicked(move |_| {
            let mut this = this_clone.borrow_mut();

            let context = this.context.take().unwrap();

            let exporter = toc::Factory::get_exporter(
                this.get_selected_format()
            );

            let media_path = context.path.clone();
            let target_path = media_path.with_file_name(&format!("{}.{}",
                media_path.file_stem()
                    .expect("ExportController::export_btn clicked, failed to get file_stem")
                    .to_str()
                    .expect("ExportController::export_btn clicked, failed to get file_stem as str"),
                exporter.extension(),
            ));

            // TODO: handle file related errors
            let mut output_file = File::create(target_path)
                .expect("ExportController::export_btn clicked couldn't create output file");

            {
                let info = context.info.lock()
                    .expect(
                        "ExportController::export_btn clicked, failed to lock media info",
                    );
                exporter.write(
                    &info.metadata,
                    &info.chapters,
                    &mut output_file
                );
            }

            main_ctrl_clone.borrow_mut().restore_context(context);

            this.export_dlg.hide();
        });
    }

    pub fn open(&mut self, context: Context) {
        self.context = Some(context);
        self.export_dlg.present();
    }

    fn get_selected_format(&self) -> toc::Format {
        if self.mkvmerge_txt_rdbtn.get_active() {
            toc::Format::MKVMergeText
        } else if self.cue_rdbtn.get_active() {
            toc::Format::CueSheet
        } else {
            unreachable!("ExportController::get_selected_format no selected radio button");
        }
    }
}
