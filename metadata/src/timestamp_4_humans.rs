use nom::{
    branch::alt,
    bytes::complete::tag,
    combinator::opt,
    error::{Error, ErrorKind},
    sequence::{preceded, separated_pair, tuple},
    Err, IResult,
};

use std::{fmt, string::ToString};

use super::{parse_to, Duration};

pub fn parse_timestamp(i: &str) -> IResult<&str, Timestamp4Humans> {
    let mut parse_timestamp_ = tuple((
        separated_pair(parse_to::<u8>, tag(":"), parse_to::<u8>),
        opt(tuple((
            // the next tag determines whether the 1st number is h or mn
            alt((tag(":"), tag("."))),
            parse_to::<u16>,
            opt(preceded(tag("."), parse_to::<u16>)),
        ))),
    ));

    let (i, res) = parse_timestamp_(i)?;

    let ts = match res {
        ((h, m), Some((":", s, ms))) => {
            let s = u8::try_from(s).map_err(|_| Err::Error(Error::new(i, ErrorKind::Digit)))?;

            Timestamp4Humans {
                h,
                m,
                s,
                ms: ms.unwrap_or(0),
                ..Timestamp4Humans::default()
            }
        }
        ((m, s), Some((".", ms, us))) => Timestamp4Humans {
            h: 0,
            m,
            s,
            ms,
            us: us.unwrap_or(0),
            ..Timestamp4Humans::default()
        },
        ((_, _), Some((_, _, _))) => unreachable!("unexpected separator returned by parser"),
        ((h, m), None) => Timestamp4Humans {
            h,
            m,
            ..Timestamp4Humans::default()
        },
    };

    Ok((i, ts))
}

#[test]
fn parse_string() {
    let ts_res = parse_timestamp("11:42:20.010");
    assert!(ts_res.is_ok());
    let ts = ts_res.unwrap().1;
    assert_eq!(ts.h, 11);
    assert_eq!(ts.m, 42);
    assert_eq!(ts.s, 20);
    assert_eq!(ts.ms, 10);
    assert_eq!(ts.us, 0);
    assert_eq!(ts.nano, 0);
    assert_eq!(
        ts.nano_total(),
        ((((11 * 60 + 42) * 60 + 20) * 1_000) + 10) * 1_000 * 1_000
    );

    let ts_res = parse_timestamp("42:20.010");
    assert!(ts_res.is_ok());
    let ts = ts_res.unwrap().1;
    assert_eq!(ts.h, 0);
    assert_eq!(ts.m, 42);
    assert_eq!(ts.s, 20);
    assert_eq!(ts.ms, 10);
    assert_eq!(ts.us, 0);
    assert_eq!(ts.nano, 0);
    assert_eq!(
        ts.nano_total(),
        (((42 * 60 + 20) * 1_000) + 10) * 1_000 * 1_000
    );

    let ts_res = parse_timestamp("42:20.010.015");
    assert!(ts_res.is_ok());
    let ts = ts_res.unwrap().1;
    assert_eq!(ts.h, 0);
    assert_eq!(ts.m, 42);
    assert_eq!(ts.s, 20);
    assert_eq!(ts.ms, 10);
    assert_eq!(ts.us, 15);
    assert_eq!(ts.nano, 0);
    assert_eq!(
        ts.nano_total(),
        ((((42 * 60 + 20) * 1_000) + 10) * 1_000 + 15) * 1_000
    );

    assert!(parse_timestamp("abc:15").is_err());
    assert!(parse_timestamp("42:aa.015").is_err());

    let ts_res = parse_timestamp("42:20a");
    assert!(ts_res.is_ok());
    let (i, _) = ts_res.unwrap();
    assert_eq!("a", i);
}

#[derive(Default)]
pub struct Timestamp4Humans {
    pub nano: u16,
    pub us: u16,
    pub ms: u16,
    pub s: u8,
    pub m: u8,
    pub h: u8,
}

// New type to force display of hours
pub struct Timestamp4HumansWithHours(Timestamp4Humans);
// New type to force display of micro seconds
pub struct Timestamp4HumansWithMicro(Timestamp4Humans);

impl Timestamp4Humans {
    pub fn from_nano(nano_total: u64) -> Self {
        let us_total = nano_total / 1_000;
        let ms_total = us_total / 1_000;
        let s_total = ms_total / 1_000;
        let m_total = s_total / 60;

        Timestamp4Humans {
            nano: (nano_total % 1_000) as u16,
            us: (us_total % 1_000) as u16,
            ms: (ms_total % 1_000) as u16,
            s: (s_total % 60) as u8,
            m: (m_total % 60) as u8,
            h: (m_total / 60) as u8,
        }
    }

    pub fn nano_total(&self) -> u64 {
        ((((u64::from(self.h) * 60 + u64::from(self.m)) * 60 + u64::from(self.s)) * 1_000
            + u64::from(self.ms))
            * 1_000
            + u64::from(self.us))
            * 1_000
            + u64::from(self.nano)
    }

    pub fn from_duration(duration: Duration) -> Self {
        Self::from_nano(duration.into())
    }

    pub fn with_hours(self) -> Timestamp4HumansWithHours {
        Timestamp4HumansWithHours(self)
    }

    pub fn with_micro(self) -> Timestamp4HumansWithMicro {
        Timestamp4HumansWithMicro(self)
    }
}

impl ToString for Timestamp4Humans {
    fn to_string(&self) -> String {
        if self.h == 0 {
            format!("{:02}:{:02}.{:03}", self.m, self.s, self.ms)
        } else {
            format!("{:02}:{:02}:{:02}.{:03}", self.h, self.m, self.s, self.ms,)
        }
    }
}

impl fmt::Debug for Timestamp4Humans {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Timestamp4Humans")
            .field(&self.to_string())
            .finish()
    }
}

impl ToString for Timestamp4HumansWithMicro {
    fn to_string(&self) -> String {
        Timestamp4Humans::to_string(&self.0) + &format!(".{:03}", self.0.us)
    }
}

impl ToString for Timestamp4HumansWithHours {
    fn to_string(&self) -> String {
        let res = Timestamp4Humans::to_string(&self.0);
        if self.0.h == 0 {
            format!("{:02}:{}", self.0.h, &res)
        } else {
            res
        }
    }
}
