//! A sandboxed `os` library: time queries only.
//!
//! The clock comes from the host through [`Lua::set_clock`], because
//! `std::time` is unavailable on `wasm32-unknown-unknown`.

use std::rc::Rc;

use super::{arg, lib, num_arg, opt_num, register, str_arg};
use crate::error::Result;
use crate::value::{Ret, Value};
use crate::vm::Lua;

/// Host-provided wall clock: seconds since the Unix epoch.
pub struct Clock(pub Box<dyn Fn() -> f64>);

fn now(lua: &Lua) -> f64 {
    if let Some(clock) = lua.app_data::<Clock>() {
        return (clock.0)();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64())
    }
    #[cfg(target_arch = "wasm32")]
    {
        0.0
    }
}

impl Lua {
    /// Installs the wall clock used by `os.time`, `os.clock` and `os.date`.
    pub fn set_clock(&mut self, clock: impl Fn() -> f64 + 'static) {
        self.set_app_data(Rc::new(Clock(Box::new(clock))));
    }
}

/// Civil date from days since the epoch (proleptic Gregorian, UTC).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
    "November", "December",
];

fn date(lua: &mut Lua, args: &[Value]) -> Result<Ret> {
    let format = if arg(args, 0).is_nil() { "%c".to_string() } else { str_arg(args, 0, "date")?.to_str().into_owned() };
    let time = opt_num(args, 1, "date", now(lua))?.floor() as i64;
    let format = format.trim_start_matches('!');
    let days = time.div_euclid(86_400);
    let seconds = time.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    let weekday = (days + 4).rem_euclid(7) as usize;
    let yday = days - days_from_civil(year, 1, 1) + 1;

    if format.starts_with("*t") {
        let t = lua.new_table();
        t.set_str("year", year);
        t.set_str("month", i64::from(month));
        t.set_str("day", i64::from(day));
        t.set_str("hour", hour);
        t.set_str("min", minute);
        t.set_str("sec", second);
        t.set_str("wday", weekday as i64 + 1);
        t.set_str("yday", yday);
        t.set_str("isdst", false);
        return Ok(Ret::One(Value::Table(t)));
    }

    let mut out = String::new();
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&year.to_string()),
            Some('y') => out.push_str(&format!("{:02}", year.rem_euclid(100))),
            Some('m') => out.push_str(&format!("{month:02}")),
            Some('d') => out.push_str(&format!("{day:02}")),
            Some('H') => out.push_str(&format!("{hour:02}")),
            Some('I') => out.push_str(&format!("{:02}", if hour % 12 == 0 { 12 } else { hour % 12 })),
            Some('M') => out.push_str(&format!("{minute:02}")),
            Some('S') => out.push_str(&format!("{second:02}")),
            Some('p') => out.push_str(if hour < 12 { "AM" } else { "PM" }),
            Some('A') => out.push_str(WEEKDAYS[weekday]),
            Some('a') => out.push_str(&WEEKDAYS[weekday][..3]),
            Some('B') => out.push_str(MONTHS[month as usize - 1]),
            Some('b') => out.push_str(&MONTHS[month as usize - 1][..3]),
            Some('j') => out.push_str(&format!("{yday:03}")),
            Some('c') => out.push_str(&format!(
                "{} {} {day:2} {hour:02}:{minute:02}:{second:02} {year}",
                &WEEKDAYS[weekday][..3],
                &MONTHS[month as usize - 1][..3]
            )),
            Some('x') => out.push_str(&format!("{month:02}/{day:02}/{:02}", year.rem_euclid(100))),
            Some('X') => out.push_str(&format!("{hour:02}:{minute:02}:{second:02}")),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    Ok(Ret::One(Value::from(out)))
}

pub(super) fn open(lua: &mut Lua) {
    let started = now(lua);
    register(
        lua,
        "os",
        lib![
            "time" => |lua, args| {
                if let Value::Table(t) = arg(args, 0) {
                    let field = |name: &str, default: i64| t.get_str(name).to_integer().unwrap_or(default);
                    let days = days_from_civil(field("year", 1970), field("month", 1), field("day", 1));
                    let seconds = field("hour", 12) * 3600 + field("min", 0) * 60 + field("sec", 0);
                    return Ok(Ret::One(Value::Int(days * 86_400 + seconds)));
                }
                Ok(Ret::One(Value::Int(now(lua).floor() as i64)))
            },
            "clock" => move |lua, _| Ok(Ret::One(Value::Num(now(lua) - started))),
            "date" => date,
            "difftime" => |_, args| Ok(Ret::One(Value::Num(
                num_arg(args, 0, "difftime")? - opt_num(args, 1, "difftime", 0.0)?,
            ))),
        ],
    );
}
