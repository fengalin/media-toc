use std::fmt;

#[derive(Clone, Copy)]
pub struct Timestamp {
    pub nano: u64,
    pub us: u64,
    pub ms: u64,
    pub s: u64,
    pub m: u64,
    pub h: u64,
}


impl Timestamp {
    pub fn format(nano_total: u64) -> String {
        let us_total = nano_total / 1_000;
        let ms_total = us_total / 1_000;
        let s_total = ms_total / 1_000;
        let m_total = s_total / 60;
        let h = m_total / 60;

        if h == 0 {
            format!("{:02}:{:02}.{:03}",
                m_total % 60, s_total % 60, ms_total % 1_000
            ).to_owned()
        } else {
            format!("{:02}:{:02}:{:02}.{:03}",
                h, m_total % 60, s_total % 60, ms_total % 1_000
            ).to_owned()
        }
    }

    pub fn from_nano(nano_total: u64) -> Self {
        let us_total = nano_total / 1_000;
        let ms_total = us_total / 1_000;
        let s_total = ms_total / 1_000;
        let m_total = s_total / 60;

        Timestamp {
            nano: nano_total % 1_000,
            us: us_total % 1_000,
            ms: ms_total % 1_000,
            s: s_total % 60,
            m: m_total % 60,
            h: m_total / 60,
        }
    }

    pub fn from_signed_nano(nano: i64) -> Self {
        if nano.is_negative() {
            Timestamp {
                nano: 0,
                us: 0,
                ms: 0,
                s: 0,
                m: 0,
                h: 0,
            }
        } else {
            Timestamp::from_nano(nano as u64)
        }
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let prefix = if self.h > 0 {
            format!("{:02}:", self.h).to_owned()
        }
        else {
            String::new()
        };

        write!(f, "{}{:02}:{:02}.{:03}",
            prefix, self.m, self.s, self.ms
        )
    }
}
