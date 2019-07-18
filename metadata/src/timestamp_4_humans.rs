use nom::{
    alt, do_parse, eof, flat_map, named, opt, parse_to, tag, take_while1, types::CompleteStr,
};

use std::{fmt, string::ToString};

named!(
    parse_digits<CompleteStr<'_>, u64>,
    flat_map!(take_while1!(|c| c >= '0' && c <= '9'), parse_to!(u64))
);

named!(
    parse_opt_dot_digits<CompleteStr<'_>, Option<u64>>,
    opt!(do_parse!(tag!(".") >> nb: parse_digits >> (nb)))
);

named!(pub parse_timestamp<CompleteStr<'_>, Timestamp4Humans>,
    do_parse!(
        nb1: parse_digits >>
        tag!(":") >>
        nb2: parse_digits >>
        nb1_is_hours: opt!(alt!(
            tag!(":") => { |_| true } |
            tag!(".") => { |_| false }
        )) >>
        nb3: opt!(parse_digits) >>
        nb4: parse_opt_dot_digits >>
        nb5: parse_opt_dot_digits >>
        eof!() >>
        ({
            let mut ts = {
                if nb1_is_hours.unwrap_or(false) {
                    Timestamp4Humans {
                        h: nb1 as u8,
                        m: nb2 as u8,
                        s: nb3.unwrap_or(0) as u8,
                        ms: nb4.unwrap_or(0) as u16,
                        us: nb5.unwrap_or(0) as u16,
                        .. Timestamp4Humans::default()
                    }
                } else {
                    Timestamp4Humans {
                        h: 0u8,
                        m: nb1 as u8,
                        s: nb2 as u8,
                        ms: nb3.unwrap_or(0) as u16,
                        us: nb4.unwrap_or(0) as u16,
                        nano: nb5.unwrap_or(0) as u16,
                        .. Timestamp4Humans::default()
                    }
                }
            };
            ts.nano_total =
                (
                    (
                        (
                            (u64::from(ts.h) * 60 + u64::from(ts.m)) * 60 + u64::from(ts.s)
                        ) * 1_000 + u64::from(ts.ms)
                    ) * 1_000 + u64::from(ts.us)
                ) * 1_000 + u64::from(ts.nano);
            ts
        })
    )
);

#[test]
fn parse_string() {
    use nom;

    let ts_res = parse_timestamp(CompleteStr("11:42:20.010"));
    assert!(ts_res.is_ok());
    let ts = ts_res.unwrap().1;
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

    let ts_res = parse_timestamp(CompleteStr("42:20.010"));
    assert!(ts_res.is_ok());
    let ts = ts_res.unwrap().1;
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

    let ts_res = parse_timestamp(CompleteStr("42:20.010.015"));
    assert!(ts_res.is_ok());
    let ts = ts_res.unwrap().1;
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

    assert!(parse_timestamp(CompleteStr("abc:15")).is_err());
    assert!(parse_timestamp(CompleteStr("42:aa.015")).is_err());

    let ts_res = parse_timestamp(CompleteStr("42:20a"));
    let err = ts_res.unwrap_err();
    if let nom::Err::Error(nom::Context::Code(i, code)) = err {
        assert_eq!(CompleteStr("a"), i);
        assert_eq!(nom::ErrorKind::Eof, code);
    } else {
        panic!("unexpected error type returned");
    }
}

#[derive(Default)]
pub struct Timestamp4Humans {
    pub nano_total: u64,
    pub nano: u16,
    pub us: u16,
    pub ms: u16,
    pub s: u8,
    pub m: u8,
    pub h: u8,
}

pub struct Timestamp4HumansWithHours(Timestamp4Humans);
pub struct Timestamp4HumansWithMicro(Timestamp4Humans);

impl Timestamp4Humans {
    pub fn from_nano(nano_total: u64) -> Self {
        let us_total = nano_total / 1_000;
        let ms_total = us_total / 1_000;
        let s_total = ms_total / 1_000;
        let m_total = s_total / 60;

        Timestamp4Humans {
            nano_total,
            nano: (nano_total % 1_000) as u16,
            us: (us_total % 1_000) as u16,
            ms: (ms_total % 1_000) as u16,
            s: (s_total % 60) as u8,
            m: (m_total % 60) as u8,
            h: (m_total / 60) as u8,
        }
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
            format!(
                "{:02}:{:02}:{:02}.{:03}",
                self.h,
                self.m,
                self.s,
                self.ms,
            )
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
