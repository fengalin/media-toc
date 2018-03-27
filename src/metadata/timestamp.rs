use std::fmt;

#[derive(Clone, Copy, Default)]
pub struct Timestamp {
    pub nano_total: u64,
    pub nano: u64,
    pub us: u64,
    pub ms: u64,
    pub s: u64,
    pub m: u64,
    pub h: u64,
}

macro_rules! pop_and_parse(
    ($source:expr) => {
        match $source.pop() {
            Some(part) => match part.parse::<u64>() {
                Ok(value) => value,
                Err(_) => {
                    warn!("from_string can't parse {}", part);
                    return Err(());
                }
            },
            None => {
                warn!("from_string couldn't pop part");
                return Err(());
            }
        }
    };
);

impl Timestamp {
    pub fn new() -> Self {
        Timestamp::default()
    }

    pub fn from_nano(nano_total: u64) -> Self {
        let us_total = nano_total / 1_000;
        let ms_total = us_total / 1_000;
        let s_total = ms_total / 1_000;
        let m_total = s_total / 60;

        Timestamp {
            nano_total: nano_total,
            nano: nano_total % 1_000,
            us: us_total % 1_000,
            ms: ms_total % 1_000,
            s: s_total % 60,
            m: m_total % 60,
            h: m_total / 60,
        }
    }

    pub fn from_string(input: &str) -> Result<Self, ()> {
        let mut parts: Vec<&str> = input.trim().split(':').collect();
        if parts.len() < 2 {
            warn!("from_string can't parse {}", input);
            return Err(());
        }

        let mut ts = Timestamp::new();

        // parse last part, expecting 000.000 or 000.000.000
        let last = parts.pop().unwrap();
        let mut dot_parts: Vec<&str> = last.split('.').collect();
        ts.us = if dot_parts.len() == 3 {
            pop_and_parse!(dot_parts)
        } else {
            0
        };

        ts.ms = if dot_parts.len() == 2 {
            pop_and_parse!(dot_parts)
        } else {
            0
        };

        ts.s = pop_and_parse!(dot_parts);
        ts.m = pop_and_parse!(parts);

        if parts.is_empty() {
            ts.h = 0;
        } else if parts.len() == 1 {
            ts.h = pop_and_parse!(parts);
        } else {
            warn!("from_string too many parts in {}", input);
            return Err(());
        }

        ts.nano_total =
            ((((ts.h * 60 + ts.m) * 60 + ts.s) * 1_000 + ts.ms) * 1_000 + ts.us) * 1_000;

        Ok(ts)
    }

    pub fn format_with_hours(&self) -> String {
        format!("{:02}:{:02}:{:02}.{:03}", self.h, self.m, self.s, self.ms).to_owned()
    }

    pub fn format(nano_total: u64, with_micro: bool) -> String {
        let us_total = nano_total / 1_000;
        let ms_total = us_total / 1_000;
        let s_total = ms_total / 1_000;
        let m_total = s_total / 60;
        let h = m_total / 60;

        let micro = if with_micro {
            format!(".{:03}", us_total % 1_000)
        } else {
            "".to_owned()
        };
        if h == 0 {
            format!(
                "{:02}:{:02}.{:03}{}",
                m_total % 60,
                s_total % 60,
                ms_total % 1_000,
                micro
            ).to_owned()
        } else {
            format!(
                "{:02}:{:02}:{:02}.{:03}{}",
                h,
                m_total % 60,
                s_total % 60,
                ms_total % 1_000,
                micro
            ).to_owned()
        }
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let prefix = if self.h > 0 {
            format!("{:02}:", self.h).to_owned()
        } else {
            String::new()
        };

        write!(f, "{}{:02}:{:02}.{:03}", prefix, self.m, self.s, self.ms)
    }
}

#[cfg(test)]
mod tests {
    //use env_logger;
    use metadata::Timestamp;

    #[test]
    fn parse_string() {
        //env_logger::try_init();

        let ts = Timestamp::from_string("11:42:20.010");
        assert!(ts.is_ok());
        let ts = ts.unwrap();
        assert_eq!(ts.h, 11);
        assert_eq!(ts.m, 42);
        assert_eq!(ts.s, 20);
        assert_eq!(ts.ms, 10);
        assert_eq!(ts.us, 0);
        assert_eq!(ts.nano, 0);
        assert_eq!(
            ts.nano_total,
            ((((11 * 60 + 42) * 60 + 20) * 1_000) + 10) * 1_000 * 1_000
        );

        let ts = Timestamp::from_string("42:20.010");
        assert!(ts.is_ok());
        let ts = ts.unwrap();
        assert_eq!(ts.h, 0);
        assert_eq!(ts.m, 42);
        assert_eq!(ts.s, 20);
        assert_eq!(ts.ms, 10);
        assert_eq!(ts.us, 0);
        assert_eq!(ts.nano, 0);
        assert_eq!(
            ts.nano_total,
            (((42 * 60 + 20) * 1_000) + 10) * 1_000 * 1_000
        );

        let ts = Timestamp::from_string("42:20.010.015");
        assert!(ts.is_ok());
        let ts = ts.unwrap();
        assert_eq!(ts.h, 0);
        assert_eq!(ts.m, 42);
        assert_eq!(ts.s, 20);
        assert_eq!(ts.ms, 10);
        assert_eq!(ts.us, 15);
        assert_eq!(ts.nano, 0);
        assert_eq!(
            ts.nano_total,
            ((((42 * 60 + 20) * 1_000) + 10) * 1_000 + 15) * 1_000
        );

        assert!(Timestamp::from_string("abc.015").is_err());
        assert!(Timestamp::from_string("42:aa.015").is_err());
        assert!(Timestamp::from_string("20:11:42:010.015").is_err());
    }
}
