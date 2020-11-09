use gtk::prelude::*;

use log::debug;

use std::{
    cell::RefCell,
    collections::Bound::Included,
    rc::Rc,
    sync::{Arc, Mutex},
};

use media::{SampleIndexRange, Timestamp};
use metadata::Duration;
use renderers::{ImagePositions, WaveformRenderer, BACKGROUND_COLOR};

use crate::info::{self, ChaptersBoundaries};

// Use this text to compute the largest text box for the waveform limits
// This is required to position the labels in such a way they don't
// move constantly depending on the digits width
const LIMIT_TEXT_MN: &str = "00:00.000";
const LIMIT_TEXT_H: &str = "00:00:00.000";
const CURSOR_TEXT_MN: &str = "00:00.000.000";
const CURSOR_TEXT_H: &str = "00:00:00.000.000";

// Other UI components refresh period
const OTHER_UI_REFRESH_PERIOD: Duration = Duration::from_millis(50);

const ONE_HOUR: Duration = Duration::from_secs(60 * 60);

#[derive(Default)]
struct TextMetrics {
    font_family: Option<String>,
    font_size: f64,
    twice_font_size: f64,
    half_font_size: f64,
    limit_mn_width: f64,
    limit_h_width: f64,
    limit_y: f64,
    cursor_mn_width: f64,
    cursor_h_width: f64,
    cursor_y: f64,
    ref_lbl: Option<gtk::Label>,
}

impl TextMetrics {
    fn new(ref_lbl: gtk::Label) -> Self {
        TextMetrics {
            ref_lbl: Some(ref_lbl),
            ..Default::default()
        }
    }

    fn set_text_metrics(&mut self, cr: &cairo::Context) {
        // FIXME use Once for this
        match self.font_family {
            Some(ref family) => {
                cr.select_font_face(family, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
                cr.set_font_size(self.font_size);
            }
            None => {
                // Get font specs from the reference label
                let ref_layout = self.ref_lbl.as_ref().unwrap().get_layout().unwrap();
                let ref_ctx = ref_layout.get_context().unwrap();
                let font_desc = ref_ctx.get_font_description().unwrap();

                let family = font_desc.get_family().unwrap();
                cr.select_font_face(&family, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
                let font_size = f64::from(ref_layout.get_baseline() / pango::SCALE);
                cr.set_font_size(font_size);

                self.font_family = Some(family.to_string());
                self.font_size = font_size;
                self.twice_font_size = 2f64 * font_size;
                self.half_font_size = 0.5f64 * font_size;

                self.limit_mn_width = cr.text_extents(LIMIT_TEXT_MN).width;
                self.limit_h_width = cr.text_extents(LIMIT_TEXT_H).width;
                self.limit_y = 2f64 * font_size;
                self.cursor_mn_width = cr.text_extents(CURSOR_TEXT_MN).width;
                self.cursor_h_width = cr.text_extents(CURSOR_TEXT_H).width;
                self.cursor_y = font_size;
            }
        }
    }
}

pub struct WaveformWithOverlay {
    waveform_renderer_mtx: Arc<Mutex<Box<WaveformRenderer>>>,
    text_metrics: TextMetrics,
    boundaries: Rc<RefCell<ChaptersBoundaries>>,
    positions: Rc<RefCell<ImagePositions>>,
    last_other_ui_refresh: Timestamp,
}

impl WaveformWithOverlay {
    pub fn new(
        waveform_renderer_mtx: &Arc<Mutex<Box<WaveformRenderer>>>,
        positions: &Rc<RefCell<ImagePositions>>,
        boundaries: &Rc<RefCell<ChaptersBoundaries>>,
        ref_lbl: &gtk::Label,
    ) -> Self {
        WaveformWithOverlay {
            waveform_renderer_mtx: Arc::clone(waveform_renderer_mtx),
            text_metrics: TextMetrics::new(ref_lbl.clone()),
            boundaries: Rc::clone(boundaries),
            positions: Rc::clone(positions),
            last_other_ui_refresh: Timestamp::default(),
        }
    }

    pub fn draw(&mut self, da: &gtk::DrawingArea, cr: &cairo::Context) {
        cr.set_source_rgb(BACKGROUND_COLOR.0, BACKGROUND_COLOR.1, BACKGROUND_COLOR.2);
        cr.paint();

        let (positions, state) = {
            let waveform_renderer = &mut *self.waveform_renderer_mtx.lock().unwrap();
            // FIXME send an event?
            //self.playback_needs_refresh = waveform_renderer.playback_needs_refresh();

            if let Err(err) = waveform_renderer.refresh() {
                if err.is_not_ready() {
                    return;
                } else {
                    panic!(err);
                }
            }

            let (image, positions, state) = match waveform_renderer.image() {
                Some(image_and_positions) => image_and_positions,
                None => {
                    debug!("draw got no image");
                    return;
                }
            };

            image.with_surface_external_context(cr, |cr, surface| {
                cr.set_source_surface(surface, -positions.offset.x, 0f64);
                cr.paint();
            });

            (positions, state)
        };

        cr.scale(1f64, 1f64);
        cr.set_source_rgb(1f64, 1f64, 0f64);

        self.text_metrics.set_text_metrics(cr);

        // first position
        let first_text = positions.offset.ts.for_humans().to_string();
        let first_text_end = if positions.offset.ts < ONE_HOUR {
            2f64 + self.text_metrics.limit_mn_width
        } else {
            2f64 + self.text_metrics.limit_h_width
        };
        cr.move_to(2f64, self.text_metrics.twice_font_size);
        cr.show_text(&first_text);

        // last position
        let last_text = positions.last.ts.for_humans().to_string();
        let last_text_start = if positions.last.ts < ONE_HOUR {
            2f64 + self.text_metrics.limit_mn_width
        } else {
            2f64 + self.text_metrics.limit_h_width
        };
        if positions.last.x - last_text_start > first_text_end + 5f64 {
            // last text won't overlap with first text
            cr.move_to(
                positions.last.x - last_text_start,
                self.text_metrics.twice_font_size,
            );
            cr.show_text(&last_text);
        }

        // Draw in-range chapters boundaries
        let boundaries = self.boundaries.borrow();

        let chapter_range =
            boundaries.range((Included(&positions.offset.ts), Included(&positions.last.ts)));

        let allocation = da.get_allocation();
        let (area_width, area_height) = (allocation.width as f64, allocation.width as f64);

        cr.set_source_rgb(0.5f64, 0.6f64, 1f64);
        cr.set_line_width(1f64);
        let boundary_y0 = self.text_metrics.twice_font_size + 5f64;
        let text_base = allocation.height as f64 - self.text_metrics.half_font_size;

        for (boundary, chapters) in chapter_range {
            if *boundary >= positions.offset.ts {
                let x = SampleIndexRange::from_duration(
                    *boundary - positions.offset.ts,
                    positions.sample_duration,
                )
                .as_f64()
                    / positions.sample_step;
                cr.move_to(x, boundary_y0);
                cr.line_to(x, area_height);
                cr.stroke();

                if let Some(ref prev_chapter) = chapters.prev {
                    cr.move_to(
                        x - 5f64 - cr.text_extents(&prev_chapter.title).width,
                        text_base,
                    );
                    cr.show_text(&prev_chapter.title);
                }

                if let Some(ref next_chapter) = chapters.next {
                    cr.move_to(x + 5f64, text_base);
                    cr.show_text(&next_chapter.title);
                }
            }
        }

        if let Some(cursor) = &positions.cursor {
            // draw current pos
            cr.set_source_rgb(1f64, 1f64, 0f64);

            let cursor_text = cursor.ts.for_humans().with_micro().to_string();
            let cursor_text_end = if cursor.ts < ONE_HOUR {
                5f64 + self.text_metrics.cursor_mn_width
            } else {
                5f64 + self.text_metrics.cursor_h_width
            };
            let cursor_text_x = if cursor.x + cursor_text_end < area_width {
                cursor.x + 5f64
            } else {
                cursor.x - cursor_text_end
            };
            cr.move_to(cursor_text_x, self.text_metrics.font_size);
            cr.show_text(&cursor_text);

            cr.set_line_width(1f64);
            cr.move_to(cursor.x, 0f64);
            cr.line_to(cursor.x, area_height - self.text_metrics.twice_font_size);
            cr.stroke();

            let cursor_ts = cursor.ts;

            // update other UI position
            // Note: we go through the audio controller here in order
            // to reduce position queries on the ref gst element
            if !state.is_playing()
                || cursor.ts < self.last_other_ui_refresh
                || cursor.ts > self.last_other_ui_refresh + OTHER_UI_REFRESH_PERIOD
            {
                info::refresh(cursor_ts);
                self.last_other_ui_refresh = cursor_ts;
            }
        }

        *self.positions.borrow_mut() = positions;
    }
}
