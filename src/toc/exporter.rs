use std::io::Write;

use super::Chapter;

pub trait Exporter {
    fn extension(&self) -> &'static str;
    fn write(&self, toc: &[Chapter], destination: &mut Write);
}
