use nom::{
    alt, do_parse, eof, flat_map, named, opt, parse_to, tag, take_while1, types::CompleteStr,
};

use std::fmt;

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
                            (ts.h as u64 * 60 + ts.m as u64) * 60 + ts.s as u64
                        ) * 1_000 + ts.ms as u64
                    ) * 1_000 + ts.us as u64
                ) * 1_000 + ts.nano as u64;
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

    pub fn format_with_hours(&self) -> String {
        format!("{:02}:{:02}:{:02}.{:03}", self.h, self.m, self.s, self.ms).to_owned()
    }

    pub fn as_string(&self, with_micro: bool) -> String {
        Timestamp4Humans::format(self.nano_total, with_micro)
    }

    // FIXME: use an enum for with_micro
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
            )
            .to_owned()
        } else {
            format!(
                "{:02}:{:02}:{:02}.{:03}{}",
                h,
                m_total % 60,
                s_total % 60,
                ms_total % 1_000,
                micro
            )
            .to_owned()
        }
    }
}

impl fmt::Display for Timestamp4Humans {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix = if self.h > 0 {
            format!("{:02}:", self.h).to_owned()
        } else {
            String::new()
        };

        write!(f, "{}{:02}:{:02}.{:03}", prefix, self.m, self.s, self.ms)
    }
}

impl fmt::Debug for Timestamp4Humans {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Timestamp4Humans")
            .field(&self.to_string())
            .finish()
    }
}
