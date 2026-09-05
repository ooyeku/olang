use crate::ast::Value;
use crate::native::{NativeHandle, NativeObject};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Weekday};
use std::collections::HashMap;

/// Error types for date operations
#[derive(Debug, thiserror::Error)]
pub enum DateError {
    #[error("Parse error: {message}")]
    ParseError { message: String },
    #[error("Invalid date: {message}")]
    InvalidDate { message: String },
    #[error("Invalid argument: {message}")]
    InvalidArgument { message: String },
    #[error("Format error: {message}")]
    FormatError { message: String },
}

/// A *misused* call — wrong arity, wrong argument type. Marked with its
/// own error type so `call_dates_function` can re-raise it instead of
/// wrapping it into `Value::Err`: rule 3 of the stdlib conventions says a
/// caller's bug must abort, never come back as a handleable `Result`
/// (`unwrap_or(dates.add_days(d), fallback)` must not swallow a typo'd
/// call). This module violated that rule wholesale until 0.68.
#[derive(Debug)]
struct Misuse(String);

impl std::fmt::Display for Misuse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Misuse {}

fn misuse(msg: impl Into<String>) -> Box<dyn std::error::Error> {
    Box::new(Misuse(msg.into()))
}

/// A calendar date as a first-class value: `typeof` says `Date`, display
/// is ISO (`2026-08-22`), equality is by date, and the `dates` OvmModule
/// below gives it ordering (`<` etc.), `date - date` (days between), and
/// `date + n` / `date - n` (day arithmetic) on both tiers.
#[derive(Debug)]
pub struct DateObject(pub NaiveDate);

impl NativeObject for DateObject {
    fn module(&self) -> &'static str {
        "dates"
    }
    fn type_name(&self) -> &'static str {
        "Date"
    }
    fn display(&self) -> String {
        self.0.to_string()
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<DateObject>()
            .map(|o| self.0 == o.0)
            .unwrap_or(false)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Wrap a NaiveDate as an olang value.
pub fn date_value(d: NaiveDate) -> Value {
    Value::Native(NativeHandle::new(DateObject(d)))
}

fn as_date_object(v: &Value) -> Option<NaiveDate> {
    match v {
        Value::Native(h) => h.0.as_any().downcast_ref::<DateObject>().map(|d| d.0),
        _ => None,
    }
}

/// The operator surface for Date values, shared by both tiers through
/// `native::binary_op_hook`. Comparisons order chronologically;
/// `date - date` is the signed day difference; `date ± int` shifts by
/// days. Anything else declines, falling through to structural equality
/// (for `==`/`!=`) or the ordinary type error.
pub struct DatesModule;

impl crate::native::OvmModule for DatesModule {
    fn name(&self) -> &'static str {
        "dates"
    }
    // The `dates` namespace is registered by the stdlib (this module's
    // builtins dispatch through the `dates.` prefix before the native
    // registry is consulted); this OvmModule exists purely to give Date
    // values operators.
    fn namespaces(&self) -> Vec<(String, Value)> {
        Vec::new()
    }
    fn dispatch(&self, func: &str, _args: Vec<Value>) -> Result<Value, String> {
        Err(format!(
            "dates.{func} dispatches through the stdlib, not the native registry"
        ))
    }
    fn binary_op(
        &self,
        op: &crate::ast::BinaryOp,
        lhs: &Value,
        rhs: &Value,
    ) -> Option<Result<Value, String>> {
        use crate::ast::BinaryOp as B;
        match (as_date_object(lhs), as_date_object(rhs)) {
            (Some(a), Some(b)) => match op {
                B::LessThan => Some(Ok(Value::Boolean(a < b))),
                B::LessThanEqual => Some(Ok(Value::Boolean(a <= b))),
                B::GreaterThan => Some(Ok(Value::Boolean(a > b))),
                B::GreaterThanEqual => Some(Ok(Value::Boolean(a >= b))),
                B::Subtract => Some(Ok(Value::Integer(a.signed_duration_since(b).num_days()))),
                _ => None,
            },
            (Some(a), None) => match (op, rhs) {
                (B::Add, Value::Integer(n)) => Some(shift_days(a, *n)),
                (B::Subtract, Value::Integer(n)) => Some(shift_days(a, -*n)),
                _ => None,
            },
            (None, Some(b)) => match (op, lhs) {
                // `n + date` commutes; `n - date` stays a type error.
                (B::Add, Value::Integer(n)) => Some(shift_days(b, *n)),
                _ => None,
            },
            (None, None) => None,
        }
    }
}

fn shift_days(d: NaiveDate, n: i64) -> Result<Value, String> {
    let shifted = if n < 0 {
        d.checked_sub_days(chrono::Days::new(n.unsigned_abs()))
    } else {
        d.checked_add_days(chrono::Days::new(n as u64))
    };
    shifted
        .map(date_value)
        .ok_or_else(|| "Date arithmetic overflow".to_string())
}

/// Accept a date argument as either a Date value or a date string, and
/// remember which, so operations can answer in kind: Date in → Date out,
/// string in → string out (the pre-0.68 behavior, kept for compatibility).
fn date_arg(v: &Value, ctx: &str) -> Result<(NaiveDate, bool), Box<dyn std::error::Error>> {
    match v {
        Value::Native(h) => match h.0.as_any().downcast_ref::<DateObject>() {
            Some(d) => Ok((d.0, true)),
            None => Err(misuse(format!(
                "{ctx} must be a Date or a date string, got a {} handle",
                h.0.type_name()
            ))),
        },
        Value::String(s) => parse_date_flexible(s).map(|d| (d, false)),
        other => Err(misuse(format!(
            "{ctx} must be a Date or a date string, got {}",
            other.type_name()
        ))),
    }
}

/// Answer in the caller's own kind — see `date_arg`.
fn date_out(d: NaiveDate, native: bool) -> Value {
    if native {
        date_value(d)
    } else {
        Value::String(d.to_string().into())
    }
}

/// Creates the dates module with all date and time functions
pub fn create_dates_module() -> Value {
    let mut module = HashMap::new();

    // Current date/time functions
    module.insert("now".to_string(), create_builtin_function("now", 0));
    module.insert("utc_now".to_string(), create_builtin_function("utc_now", 0));
    module.insert("stamp".to_string(), create_builtin_function("stamp", 0));
    module.insert(
        "stamp_ms".to_string(),
        create_builtin_function("stamp_ms", 0),
    );
    module.insert("today".to_string(), create_builtin_function("today", 0));

    // Date creation functions
    module.insert("date".to_string(), create_builtin_function("date", 3));
    module.insert(
        "datetime".to_string(),
        create_builtin_function("datetime", 6),
    );
    module.insert("time".to_string(), create_builtin_function("time", 3));

    // Parsing functions
    module.insert("parse".to_string(), create_builtin_function("parse", 1));
    module.insert(
        "parse_date".to_string(),
        create_builtin_function("parse_date", 1),
    );
    module.insert(
        "parse_datetime".to_string(),
        create_builtin_function("parse_datetime", 1),
    );
    module.insert(
        "parse_time".to_string(),
        create_builtin_function("parse_time", 1),
    );

    // Formatting functions
    module.insert(
        "format_date".to_string(),
        create_builtin_function("format_date", 2),
    );
    module.insert(
        "format_datetime".to_string(),
        create_builtin_function("format_datetime", 2),
    );
    module.insert(
        "format_time".to_string(),
        create_builtin_function("format_time", 2),
    );

    // Date arithmetic
    module.insert(
        "add_days".to_string(),
        create_builtin_function("add_days", 2),
    );
    module.insert(
        "add_weeks".to_string(),
        create_builtin_function("add_weeks", 2),
    );
    module.insert(
        "add_months".to_string(),
        create_builtin_function("add_months", 2),
    );
    module.insert(
        "add_years".to_string(),
        create_builtin_function("add_years", 2),
    );
    module.insert(
        "diff_days".to_string(),
        create_builtin_function("diff_days", 2),
    );

    // Date component extraction
    module.insert("year".to_string(), create_builtin_function("year", 1));
    module.insert("month".to_string(), create_builtin_function("month", 1));
    module.insert("day".to_string(), create_builtin_function("day", 1));
    module.insert("hour".to_string(), create_builtin_function("hour", 1));
    module.insert("minute".to_string(), create_builtin_function("minute", 1));
    module.insert("second".to_string(), create_builtin_function("second", 1));
    module.insert("weekday".to_string(), create_builtin_function("weekday", 1));

    // Utility functions
    module.insert(
        "is_leap_year".to_string(),
        create_builtin_function("is_leap_year", 1),
    );
    module.insert(
        "days_in_month".to_string(),
        create_builtin_function("days_in_month", 2),
    );
    module.insert(
        "timestamp".to_string(),
        create_builtin_function("timestamp", 1),
    );
    module.insert(
        "from_timestamp".to_string(),
        create_builtin_function("from_timestamp", 1),
    );

    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
    }
}

/// Helper function to create builtin function values
fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("dates.{}", name),
        arity,
    })
}

/// Main dispatcher for dates function calls
/// Total dates operations: no fallible input, so they return bare values.
/// Everything else takes a date/time string (or produces one) and can fail,
/// so it returns an olang `Result` — see `call_dates_function`.
fn is_total(name: &str) -> bool {
    matches!(
        name,
        "now" | "utc_now" | "stamp" | "stamp_ms" | "today" | "is_leap_year" | "days_in_month"
    )
}

/// Public entry point. Fallible operations (anything parsing a date) are
/// wrapped into an olang `Result`: success becomes `Ok(value)` and a
/// *data* failure — a malformed date, an out-of-range component,
/// arithmetic overflow — becomes `Err(message)`, so a bad date never
/// aborts the program. Total operations pass through as bare values.
///
/// A *misused* call — wrong arity, wrong argument type — is re-raised
/// (stdlib rule 3): before 0.68 it too became `Err`, which meant
/// `unwrap_or(dates.add_days(d), fallback)` silently swallowed a typo'd
/// call, the exact failure mode the convention exists to prevent.
pub fn call_dates_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    if is_total(name) {
        return dispatch_dates(name, args);
    }
    match dispatch_dates(name, args) {
        Ok(value) => Ok(Value::Ok(Box::new(value))),
        // The inner message already leads with the function name
        // ("add_days: ..."), so prefixing the module yields the
        // convention's module-qualified form: "dates.add_days: ...".
        Err(e) if e.downcast_ref::<Misuse>().is_some() => Err(format!("dates.{}", e).into()),
        Err(e) => Ok(Value::Err(Box::new(Value::String(std::sync::Arc::new(
            e.to_string(),
        ))))),
    }
}

fn dispatch_dates(name: &str, args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "now" => dates_now(args),
        "utc_now" => dates_utc_now(args),
        "stamp" => dates_stamp(args),
        "stamp_ms" => dates_stamp_ms(args),
        "today" => dates_today(args),
        "date" => dates_date(args),
        "datetime" => dates_datetime(args),
        "time" => dates_time(args),
        "parse" => dates_parse(args),
        "parse_date" => dates_parse_date(args),
        "parse_datetime" => dates_parse_datetime(args),
        "parse_time" => dates_parse_time(args),
        "format_date" => dates_format_date(args),
        "format_datetime" => dates_format_datetime(args),
        "format_time" => dates_format_time(args),
        "add_days" => dates_add_days(args),
        "add_weeks" => dates_add_weeks(args),
        "add_months" => dates_add_months(args),
        "add_years" => dates_add_years(args),
        "diff_days" => dates_diff_days(args),
        "year" => dates_year(args),
        "month" => dates_month(args),
        "day" => dates_day(args),
        "hour" => dates_hour(args),
        "minute" => dates_minute(args),
        "second" => dates_second(args),
        "weekday" => dates_weekday(args),
        "is_leap_year" => dates_is_leap_year(args),
        "days_in_month" => dates_days_in_month(args),
        "timestamp" => dates_timestamp(args),
        "from_timestamp" => dates_from_timestamp(args),
        _ => Err(format!("Unknown dates function: {}", name).into()),
    }
}

/// Parse a datetime string accepting the formats this module itself produces:
/// plain ISO (with or without fractional seconds), space-separated, and RFC3339.
fn parse_datetime_flexible(s: &str) -> Result<NaiveDateTime, Box<dyn std::error::Error>> {
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Ok(dt);
        }
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.naive_local());
    }
    Err(format!(
        "Cannot parse datetime: '{}'. Expected format: YYYY-MM-DDTHH:MM:SS or RFC3339",
        s
    )
    .into())
}

/// Parse a date string, also accepting any datetime format the module produces.
fn parse_date_flexible(s: &str) -> Result<NaiveDate, Box<dyn std::error::Error>> {
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d);
    }
    parse_datetime_flexible(s)
        .map(|dt| dt.date())
        .map_err(|_| format!("Cannot parse date: '{}'. Expected format: YYYY-MM-DD", s).into())
}

/// Current local date and time
/// Usage: dates.now() -> "2024-06-15T14:30:00+00:00"
fn dates_now(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(misuse(format!(
            "now expects 0 arguments, got {}",
            args.len()
        )));
    }

    let now = crate::clock::local_now_fixed();
    Ok(Value::String(now.to_rfc3339().into()))
}

/// Current UTC date and time
/// Usage: dates.utc_now() -> "2024-06-15T14:30:00Z"
fn dates_utc_now(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(misuse(format!(
            "utc_now expects 0 arguments, got {}",
            args.len()
        )));
    }

    let now = crate::clock::utc_now();
    Ok(Value::String(now.to_rfc3339().into()))
}

/// The storage timestamp: UTC, second precision, `Z` suffix —
/// `2026-08-31T23:40:06Z`. Sortable as text across machines and
/// offsets, and free of the fractional seconds a UI never wants.
/// Usage: dates.stamp() -> "2026-08-31T23:40:06Z"
/// dates.stamp_ms(): the storage stamp at millisecond precision — the
/// one to key a row's version on. `stamp` (seconds) is the grain a person
/// reads; two edits inside one second are identical under it, which
/// makes optimistic concurrency pass a stale write.
fn dates_stamp_ms(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(misuse(format!(
            "stamp_ms expects 0 arguments, got {}",
            args.len()
        )));
    }
    let now = crate::clock::utc_now();
    Ok(Value::String(
        now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string().into(),
    ))
}

fn dates_stamp(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(misuse(format!(
            "stamp expects 0 arguments, got {}",
            args.len()
        )));
    }
    let now = crate::clock::utc_now();
    Ok(Value::String(
        now.format("%Y-%m-%dT%H:%M:%SZ").to_string().into(),
    ))
}

/// Current local date
/// Usage: dates.today() -> "2024-06-15"
fn dates_today(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(misuse(format!(
            "today expects 0 arguments, got {}",
            args.len()
        )));
    }

    let today = crate::clock::local_now_fixed().date_naive();
    Ok(Value::String(today.to_string().into()))
}

/// Create a date from year, month, day
/// Usage: dates.date(2024, 6, 15) -> "2024-06-15"
fn dates_date(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Err(misuse(format!(
            "date expects 3 arguments, got {}",
            args.len()
        )));
    }

    let year = match &args[0] {
        Value::Integer(y) => *y as i32,
        _ => return Err(misuse("date: year must be an integer")),
    };

    let month = match &args[1] {
        Value::Integer(m) => *m as u32,
        _ => return Err(misuse("date: month must be an integer")),
    };

    let day = match &args[2] {
        Value::Integer(d) => *d as u32,
        _ => return Err(misuse("date: day must be an integer")),
    };

    match NaiveDate::from_ymd_opt(year, month, day) {
        // A Date value since 0.68 (was an ISO string): the payload
        // display is unchanged, and comparisons/arithmetic now work.
        Some(date) => Ok(date_value(date)),
        None => Err(format!("Invalid date: {}-{:02}-{:02}", year, month, day).into()),
    }
}

/// Parse a Date value from a date (or datetime) string — the canonical
/// constructor from text. Accepts everything this module itself emits.
/// Usage: dates.parse("2024-06-15") -> Result<Date, Error>
fn dates_parse(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse(format!(
            "parse expects 1 argument, got {}",
            args.len()
        )));
    }
    let s = match &args[0] {
        Value::String(s) => s.as_ref(),
        other => {
            return Err(misuse(format!(
                "parse: argument must be a string, got {}",
                other.type_name()
            )));
        }
    };
    parse_date_flexible(s).map(date_value)
}

/// Create a datetime from year, month, day, hour, minute, second
/// Usage: dates.datetime(2024, 6, 15, 14, 30, 0) -> "2024-06-15T14:30:00"
fn dates_datetime(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 6 {
        return Err(misuse(format!(
            "datetime expects 6 arguments, got {}",
            args.len()
        )));
    }

    let year = match &args[0] {
        Value::Integer(y) => *y as i32,
        _ => return Err(misuse("datetime: year must be an integer")),
    };

    let month = match &args[1] {
        Value::Integer(m) => *m as u32,
        _ => return Err(misuse("datetime: month must be an integer")),
    };

    let day = match &args[2] {
        Value::Integer(d) => *d as u32,
        _ => return Err(misuse("datetime: day must be an integer")),
    };

    let hour = match &args[3] {
        Value::Integer(h) => *h as u32,
        _ => return Err(misuse("datetime: hour must be an integer")),
    };

    let minute = match &args[4] {
        Value::Integer(m) => *m as u32,
        _ => return Err(misuse("datetime: minute must be an integer")),
    };

    let second = match &args[5] {
        Value::Integer(s) => *s as u32,
        _ => return Err(misuse("datetime: second must be an integer")),
    };

    let date = match NaiveDate::from_ymd_opt(year, month, day) {
        Some(d) => d,
        None => return Err(format!("Invalid date: {}-{:02}-{:02}", year, month, day).into()),
    };

    let time = match NaiveTime::from_hms_opt(hour, minute, second) {
        Some(t) => t,
        None => {
            return Err(format!("Invalid time: {:02}:{:02}:{:02}", hour, minute, second).into());
        }
    };

    let datetime = NaiveDateTime::new(date, time);
    Ok(Value::String(datetime.to_string().into()))
}

/// Create a time from hour, minute, second
/// Usage: dates.time(14, 30, 0) -> "14:30:00"
fn dates_time(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Err(misuse(format!(
            "time expects 3 arguments, got {}",
            args.len()
        )));
    }

    let hour = match &args[0] {
        Value::Integer(h) => *h as u32,
        _ => return Err(misuse("time: hour must be an integer")),
    };

    let minute = match &args[1] {
        Value::Integer(m) => *m as u32,
        _ => return Err(misuse("time: minute must be an integer")),
    };

    let second = match &args[2] {
        Value::Integer(s) => *s as u32,
        _ => return Err(misuse("time: second must be an integer")),
    };

    match NaiveTime::from_hms_opt(hour, minute, second) {
        Some(time) => Ok(Value::String(time.to_string().into())),
        None => Err(format!("Invalid time: {:02}:{:02}:{:02}", hour, minute, second).into()),
    }
}

/// Parse a date string in ISO format
/// Usage: dates.parse_date("2024-06-15") -> "2024-06-15"
fn dates_parse_date(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse(format!(
            "parse_date expects 1 argument, got {}",
            args.len()
        )));
    }

    let date_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err(misuse("parse_date: argument must be a string")),
    };

    match NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
        Ok(date) => Ok(Value::String(date.to_string().into())),
        Err(_) => Err(format!(
            "Cannot parse date: '{}'. Expected format: YYYY-MM-DD",
            date_str
        )
        .into()),
    }
}

/// Parse a datetime string in ISO format
/// Usage: dates.parse_datetime("2024-06-15T14:30:00") -> "2024-06-15T14:30:00"
fn dates_parse_datetime(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse(format!(
            "parse_datetime expects 1 argument, got {}",
            args.len()
        )));
    }

    let datetime_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err(misuse("parse_datetime: argument must be a string")),
    };

    match NaiveDateTime::parse_from_str(datetime_str, "%Y-%m-%dT%H:%M:%S") {
        Ok(datetime) => Ok(Value::String(datetime.to_string().into())),
        Err(_) => {
            // Try with RFC3339 format
            match DateTime::parse_from_rfc3339(datetime_str) {
                Ok(datetime) => Ok(Value::String(datetime.naive_local().to_string().into())),
                Err(_) => Err(format!(
                    "Cannot parse datetime: '{}'. Expected format: YYYY-MM-DDTHH:MM:SS or RFC3339",
                    datetime_str
                )
                .into()),
            }
        }
    }
}

/// Parse a time string
/// Usage: dates.parse_time("14:30:00") -> "14:30:00"
fn dates_parse_time(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse(format!(
            "parse_time expects 1 argument, got {}",
            args.len()
        )));
    }

    let time_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err(misuse("parse_time: argument must be a string")),
    };

    match NaiveTime::parse_from_str(time_str, "%H:%M:%S") {
        Ok(time) => Ok(Value::String(time.to_string().into())),
        Err(_) => Err(format!(
            "Cannot parse time: '{}'. Expected format: HH:MM:SS",
            time_str
        )
        .into()),
    }
}

/// Format a date with a custom format string
/// Usage: dates.format_date("2024-06-15", "%B %d, %Y") -> "June 15, 2024"
fn dates_format_date(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(misuse(format!(
            "format_date expects 2 arguments, got {}",
            args.len()
        )));
    }

    let (date, _) = date_arg(&args[0], "format_date: first argument")?;

    let format_str = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => return Err(misuse("format_date: second argument must be a string")),
    };

    let formatted = date.format(format_str).to_string();
    Ok(Value::String(formatted.into()))
}

/// Format a datetime with a custom format string
/// Usage: dates.format_datetime("2024-06-15T14:30:00", "%B %d, %Y at %I:%M %p") -> "June 15, 2024 at 02:30 PM"
fn dates_format_datetime(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(misuse(format!(
            "format_datetime expects 2 arguments, got {}",
            args.len()
        )));
    }

    let datetime_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err(misuse("format_datetime: first argument must be a string")),
    };

    let format_str = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => return Err(misuse("format_datetime: second argument must be a string")),
    };

    let datetime = parse_datetime_flexible(datetime_str)?;

    let formatted = datetime.format(format_str).to_string();
    Ok(Value::String(formatted.into()))
}

/// Format a time with a custom format string
/// Usage: dates.format_time("14:30:00", "%I:%M %p") -> "02:30 PM"
fn dates_format_time(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(misuse(format!(
            "format_time expects 2 arguments, got {}",
            args.len()
        )));
    }

    let time_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err(misuse("format_time: first argument must be a string")),
    };

    let format_str = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => return Err(misuse("format_time: second argument must be a string")),
    };

    let time = match NaiveTime::parse_from_str(time_str, "%H:%M:%S") {
        Ok(t) => t,
        Err(_) => {
            return Err(format!(
                "Cannot parse time: '{}'. Expected format: HH:MM:SS",
                time_str
            )
            .into());
        }
    };

    let formatted = time.format(format_str).to_string();
    Ok(Value::String(formatted.into()))
}

/// Add days to a date
/// Usage: dates.add_days("2024-06-15", 7) -> "2024-06-22"
fn dates_add_days(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(misuse(format!(
            "add_days expects 2 arguments, got {}",
            args.len()
        )));
    }

    let (date, native) = date_arg(&args[0], "add_days: first argument")?;

    let days = match &args[1] {
        Value::Integer(d) => *d,
        _ => return Err(misuse("add_days: second argument must be an integer")),
    };

    let result = if days < 0 {
        date.checked_sub_days(chrono::Days::new(days.unsigned_abs()))
    } else {
        date.checked_add_days(chrono::Days::new(days as u64))
    };

    match result {
        Some(new_date) => Ok(date_out(new_date, native)),
        None => Err("Date arithmetic overflow".into()),
    }
}

/// Add weeks to a date
/// Usage: dates.add_weeks("2024-06-15", 2) -> "2024-06-29"
fn dates_add_weeks(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(misuse(format!(
            "add_weeks expects 2 arguments, got {}",
            args.len()
        )));
    }

    let weeks = match &args[1] {
        Value::Integer(w) => w
            .checked_mul(7) // Convert weeks to days
            .ok_or("add_weeks: overflow converting weeks to days")?,
        _ => return Err(misuse("add_weeks: second argument must be an integer")),
    };

    // Reuse add_days logic
    dates_add_days(vec![args[0].clone(), Value::Integer(weeks)])
}

/// Add months to a date (approximately)
/// Usage: dates.add_months("2024-06-15", 3) -> "2024-09-15"
fn dates_add_months(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(misuse(format!(
            "add_months expects 2 arguments, got {}",
            args.len()
        )));
    }

    let (date, native) = date_arg(&args[0], "add_months: first argument")?;

    let months = match &args[1] {
        Value::Integer(m) => *m,
        _ => return Err(misuse("add_months: second argument must be an integer")),
    };

    let day = date.day();

    // Work in total months (i64) so large offsets error instead of truncating
    let total = (date.year() as i64) * 12 + (date.month() as i64 - 1);
    let total = total
        .checked_add(months)
        .ok_or("Date arithmetic overflow")?;
    let year: i32 = i32::try_from(total.div_euclid(12)).map_err(|_| "Date arithmetic overflow")?;
    let month = (total.rem_euclid(12) + 1) as i32;

    match NaiveDate::from_ymd_opt(year, month as u32, day) {
        Some(new_date) => Ok(date_out(new_date, native)),
        None => {
            // Handle cases where the day doesn't exist in the target month (e.g., Jan 31 + 1 month)
            let last_day_of_month = NaiveDate::from_ymd_opt(year, month as u32, 1)
                .and_then(|d| d.checked_add_months(chrono::Months::new(1)))
                .and_then(|d| d.checked_sub_days(chrono::Days::new(1)))
                .map(|d| d.day())
                .unwrap_or(28);

            let adjusted_day = day.min(last_day_of_month);
            match NaiveDate::from_ymd_opt(year, month as u32, adjusted_day) {
                Some(new_date) => Ok(date_out(new_date, native)),
                None => Err("Date arithmetic error".into()),
            }
        }
    }
}

/// Add years to a date
/// Usage: dates.add_years("2024-06-15", 1) -> "2025-06-15"
fn dates_add_years(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(misuse(format!(
            "add_years expects 2 arguments, got {}",
            args.len()
        )));
    }

    let years = match &args[1] {
        Value::Integer(y) => y
            .checked_mul(12) // Convert years to months
            .ok_or("add_years: overflow converting years to months")?,
        _ => return Err(misuse("add_years: second argument must be an integer")),
    };

    // Reuse add_months logic
    dates_add_months(vec![args[0].clone(), Value::Integer(years)])
}

/// Calculate difference in days between two dates
/// Usage: dates.diff_days("2024-06-22", "2024-06-15") -> 7
fn dates_diff_days(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(misuse(format!(
            "diff_days expects 2 arguments, got {}",
            args.len()
        )));
    }

    let (date1, _) = date_arg(&args[0], "diff_days: first argument")?;
    let (date2, _) = date_arg(&args[1], "diff_days: second argument")?;

    let diff = date1.signed_duration_since(date2);
    Ok(Value::Integer(diff.num_days()))
}

/// Extract year from date
/// Usage: dates.year("2024-06-15") -> 2024
fn dates_year(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse(format!(
            "year expects 1 argument, got {}",
            args.len()
        )));
    }

    let (date, _) = date_arg(&args[0], "year: argument")?;

    Ok(Value::Integer(date.year() as i64))
}

/// Extract month from date
/// Usage: dates.month("2024-06-15") -> 6
fn dates_month(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse(format!(
            "month expects 1 argument, got {}",
            args.len()
        )));
    }

    let (date, _) = date_arg(&args[0], "month: argument")?;

    Ok(Value::Integer(date.month() as i64))
}

/// Extract day from date
/// Usage: dates.day("2024-06-15") -> 15
fn dates_day(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse(format!(
            "day expects 1 argument, got {}",
            args.len()
        )));
    }

    let (date, _) = date_arg(&args[0], "day: argument")?;

    Ok(Value::Integer(date.day() as i64))
}

/// Extract hour from datetime
/// Usage: dates.hour("2024-06-15T14:30:00") -> 14
fn dates_hour(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse(format!(
            "hour expects 1 argument, got {}",
            args.len()
        )));
    }

    let datetime_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err(misuse("hour: argument must be a string")),
    };

    let datetime = parse_datetime_flexible(datetime_str)?;

    Ok(Value::Integer(datetime.hour() as i64))
}

/// Extract minute from datetime
/// Usage: dates.minute("2024-06-15T14:30:00") -> 30
fn dates_minute(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse(format!(
            "minute expects 1 argument, got {}",
            args.len()
        )));
    }

    let datetime_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err(misuse("minute: argument must be a string")),
    };

    let datetime = parse_datetime_flexible(datetime_str)?;

    Ok(Value::Integer(datetime.minute() as i64))
}

/// Extract second from datetime
/// Usage: dates.second("2024-06-15T14:30:45") -> 45
fn dates_second(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse(format!(
            "second expects 1 argument, got {}",
            args.len()
        )));
    }

    let datetime_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err(misuse("second: argument must be a string")),
    };

    let datetime = parse_datetime_flexible(datetime_str)?;

    Ok(Value::Integer(datetime.second() as i64))
}

/// Get weekday from date (0=Sunday, 1=Monday, ..., 6=Saturday)
/// Usage: dates.weekday("2024-06-15") -> 6 (Saturday)
fn dates_weekday(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse(format!(
            "weekday expects 1 argument, got {}",
            args.len()
        )));
    }

    let (date, _) = date_arg(&args[0], "weekday: argument")?;

    let weekday = match date.weekday() {
        Weekday::Sun => 0,
        Weekday::Mon => 1,
        Weekday::Tue => 2,
        Weekday::Wed => 3,
        Weekday::Thu => 4,
        Weekday::Fri => 5,
        Weekday::Sat => 6,
    };

    Ok(Value::Integer(weekday))
}

/// Check if a year is a leap year
/// Usage: dates.is_leap_year(2024) -> true
fn dates_is_leap_year(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse(format!(
            "is_leap_year expects 1 argument, got {}",
            args.len()
        )));
    }

    let year = match &args[0] {
        Value::Integer(y) => *y as i32,
        _ => return Err(misuse("is_leap_year: argument must be an integer")),
    };

    let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    Ok(Value::Boolean(is_leap))
}

/// Get number of days in a month
/// Usage: dates.days_in_month(2024, 2) -> 29
fn dates_days_in_month(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(misuse(format!(
            "days_in_month expects 2 arguments, got {}",
            args.len()
        )));
    }

    let year = match &args[0] {
        Value::Integer(y) => *y as i32,
        _ => {
            return Err(misuse(
                "days_in_month: first argument (year) must be an integer",
            ));
        }
    };

    let month = match &args[1] {
        Value::Integer(m) => *m as u32,
        _ => {
            return Err(misuse(
                "days_in_month: second argument (month) must be an integer",
            ));
        }
    };

    if !(1..=12).contains(&month) {
        return Err("days_in_month: month must be between 1 and 12".into());
    }

    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => unreachable!(),
    };

    Ok(Value::Integer(days))
}

/// Convert datetime to Unix timestamp
/// Usage: dates.timestamp("2024-06-15T14:30:00") -> 1718461800
fn dates_timestamp(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse(format!(
            "timestamp expects 1 argument, got {}",
            args.len()
        )));
    }

    let datetime_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err(misuse("timestamp: argument must be a string")),
    };

    let datetime = parse_datetime_flexible(datetime_str)?;

    let timestamp = datetime.and_utc().timestamp();
    Ok(Value::Integer(timestamp))
}

/// Convert Unix timestamp to datetime
/// Usage: dates.from_timestamp(1718461800) -> "2024-06-15T14:30:00"
fn dates_from_timestamp(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse(format!(
            "from_timestamp expects 1 argument, got {}",
            args.len()
        )));
    }

    let timestamp = match &args[0] {
        Value::Integer(ts) => *ts,
        _ => return Err(misuse("from_timestamp: argument must be an integer")),
    };

    match DateTime::from_timestamp(timestamp, 0) {
        Some(datetime) => Ok(Value::String(datetime.naive_utc().to_string().into())),
        None => Err(format!("Invalid timestamp: {}", timestamp).into()),
    }
}
