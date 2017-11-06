extern crate gtk;
use gtk::{ButtonExt, GtkWindowExt, WidgetExt};

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
            println!("exporting {}", this.context.as_ref().unwrap().file_name);

            {
                let context = this.context.as_ref()
                    .unwrap();
                let info = context.info.lock()
                    .expect(
                        "ExportController::export_btn clicked, failed to lock media info",
                    );
                for ref chapter in &info.chapters {
                    println!("\tchapter: {}, {}, {}, {}",
                        chapter.id,
                        chapter.title(),
                        chapter.start,
                        chapter.end,
                    );
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
