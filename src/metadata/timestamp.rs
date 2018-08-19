use nom::types::CompleteStr;
use nom::{alt, call, do_parse, eof, error_position, flat_map, named, opt, parse_to, tag,
          take_while1};

use std::fmt;

named!(parse_digits<CompleteStr<'_>, u64>,
    flat_map!(
        take_while1!(|c| c >= '0' && c <= '9'),
        parse_to!(u64)
    )
);

named!(parse_opt_dot_digits<CompleteStr<'_>, Option<u64>>,
    opt!(do_parse!(
        tag!(".") >>
        nb: parse_digits >>
        (nb)
    ))
);

named!(pub parse_timestamp<CompleteStr<'_>, Timestamp>,
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
                    Timestamp {
                        h: nb1,
                        m: nb2,
                        s: nb3.unwrap_or(0),
                        ms: nb4.unwrap_or(0),
                        us: nb5.unwrap_or(0),
                        .. Timestamp::default()
                    }
                } else {
                    Timestamp {
                        h: 0,
                        m: nb1,
                        s: nb2,
                        ms: nb3.unwrap_or(0),
                        us: nb4.unwrap_or(0),
                        nano: nb5.unwrap_or(0),
                        .. Timestamp::default()
                    }
                }
            };
            ts.nano_total =
                ((((ts.h * 60 + ts.m) * 60 + ts.s) * 1_000 + ts.ms) * 1_000 + ts.us) * 1_000
                + ts.nano;
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

impl Timestamp {
    pub fn from_nano(nano_total: u64) -> Self {
        let us_total = nano_total / 1_000;
        let ms_total = us_total / 1_000;
        let s_total = ms_total / 1_000;
        let m_total = s_total / 60;

        Timestamp {
            nano_total,
            nano: nano_total % 1_000,
            us: us_total % 1_000,
            ms: ms_total % 1_000,
            s: s_total % 60,
            m: m_total % 60,
            h: m_total / 60,
        }
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix = if self.h > 0 {
            format!("{:02}:", self.h).to_owned()
        } else {
            String::new()
        };

        write!(f, "{}{:02}:{:02}.{:03}", prefix, self.m, self.s, self.ms)
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Timestamp").field(&self.to_string()).finish()
    }
}
