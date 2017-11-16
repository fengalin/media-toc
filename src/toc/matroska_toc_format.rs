static EXTENSION: &'static str = "toc.mkv";

pub struct MatroskaTocFormat {
}

impl MatroskaTocFormat {
    pub fn get_extension() -> &'static str {
        &EXTENSION
    }

    pub fn new_as_boxed() -> Box<Self> {
        Box::new(MatroskaTocFormat{})
    }
}
