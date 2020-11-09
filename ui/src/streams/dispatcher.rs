use futures::{future::LocalBoxFuture, prelude::*};

use gtk::prelude::*;

use log::debug;

use crate::{main, prelude::*, streams};

pub struct Dispatcher;
impl UIDispatcher for Dispatcher {
    type Controller = streams::Controller;
    type Event = streams::Event;

    fn setup(streams: &mut streams::Controller, _app: &gtk::Application) {
        streams
            .video
            .treeview
            .connect_cursor_changed(|_| streams::stream_clicked(gst::StreamType::VIDEO));
        streams
            .video
            .export_renderer()
            .connect_toggled(|_, tree_path| {
                streams::export_toggled(gst::StreamType::VIDEO, tree_path)
            });

        streams
            .audio
            .treeview
            .connect_cursor_changed(|_| streams::stream_clicked(gst::StreamType::AUDIO));
        streams
            .audio
            .export_renderer()
            .connect_toggled(|_, tree_path| {
                streams::export_toggled(gst::StreamType::AUDIO, tree_path)
            });

        streams
            .text
            .treeview
            .connect_cursor_changed(|_| streams::stream_clicked(gst::StreamType::TEXT));
        streams
            .text
            .export_renderer()
            .connect_toggled(|_, tree_path| {
                streams::export_toggled(gst::StreamType::TEXT, tree_path)
            });

        streams
            .page
            .connect_map(|_| main::switch_to(UIFocusContext::StreamsPage));
    }

    fn handle_event(
        main_ctrl: &mut main::Controller,
        event: impl Into<Self::Event>,
    ) -> LocalBoxFuture<'_, ()> {
        let event = event.into();
        async move {
            use streams::Event::*;

            debug!("handling {:?}", event);
            match event {
                StreamClicked(type_) => {
                    if let streams::ClickedStatus::Changed = main_ctrl.streams.stream_clicked(type_)
                    {
                        let streams = main_ctrl.streams.selected_streams();
                        main_ctrl.select_streams(&streams);
                    }
                }
                ExportToggled(type_, tree_path) => {
                    if let Some((stream_id, must_export)) =
                        main_ctrl.streams.export_toggled(type_, tree_path)
                    {
                        if let Some(pipeline) = main_ctrl.pipeline.as_mut() {
                            pipeline
                                .info
                                .write()
                                .unwrap()
                                .streams
                                .collection_mut(type_)
                                .get_mut(stream_id)
                                .as_mut()
                                .unwrap()
                                .must_export = must_export;
                        }
                    }
                }
            }
        }
        .boxed_local()
    }
}
