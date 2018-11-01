use gstreamer_audio as gst_audio;
use gstreamer_audio::AudioChannelPosition as Position;

// Inline length for `SmallVec`s with channels.
pub const INLINE_CHANNELS: usize = 8;

pub enum AudioChannelSide {
    Center,
    Left,
    NotLocalized,
    Right,
}

pub struct AudioChannel {
    pub side: AudioChannelSide,
    pub factor: f64,
}

impl AudioChannel {
    pub fn new(position: gst_audio::AudioChannelPosition) -> Self {
        let (side, factor) = match position {
            Position::Mono => (AudioChannelSide::Left, 0.9f64),
            Position::FrontLeft => (AudioChannelSide::Left, 0.9f64),
            Position::FrontRight => (AudioChannelSide::Right, 0.9f64),
            Position::FrontCenter => (AudioChannelSide::Center, 0.9f64),
            Position::Lfe1 => (AudioChannelSide::NotLocalized, 0.75f64),
            Position::RearLeft => (AudioChannelSide::Left, 0.5f64),
            Position::RearRight => (AudioChannelSide::Right, 0.5f64),
            Position::FrontLeftOfCenter => (AudioChannelSide::Left, 0.7f64),
            Position::FrontRightOfCenter => (AudioChannelSide::Right, 0.7f64),
            Position::RearCenter => (AudioChannelSide::Center, 0.5f64),
            Position::Lfe2 => (AudioChannelSide::NotLocalized, 0.72f64),
            Position::SideLeft => (AudioChannelSide::Left, 0.66f64),
            Position::SideRight => (AudioChannelSide::Right, 0.66f64),
            Position::TopFrontLeft => (AudioChannelSide::Left, 0.75f64),
            Position::TopFrontRight => (AudioChannelSide::Right, 0.75f64),
            Position::TopFrontCenter => (AudioChannelSide::Center, 0.75f64),
            Position::TopCenter => (AudioChannelSide::Center, 0.66f64),
            Position::TopRearLeft => (AudioChannelSide::Left, 0.4f64),
            Position::TopRearRight => (AudioChannelSide::Right, 0.4f64),
            Position::TopSideLeft => (AudioChannelSide::Left, 0.6f64),
            Position::TopSideRight => (AudioChannelSide::Right, 0.6f64),
            Position::TopRearCenter => (AudioChannelSide::Center, 0.4f64),
            Position::BottomFrontCenter => (AudioChannelSide::Center, 0.72f64),
            Position::BottomFrontLeft => (AudioChannelSide::Left, 0.72f64),
            Position::BottomFrontRight => (AudioChannelSide::Right, 0.72f64),
            Position::WideLeft => (AudioChannelSide::Left, 0.68f64),
            Position::WideRight => (AudioChannelSide::Right, 0.68f64),
            Position::SurroundLeft => (AudioChannelSide::Left, 0.45f64),
            Position::SurroundRight => (AudioChannelSide::Right, 0.45f64),
            _ => (AudioChannelSide::NotLocalized, 0.7f64),
        };

        AudioChannel { side, factor }
    }
}
