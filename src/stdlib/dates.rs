use crate::ast::Value;
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

/// Creates the dates module with all date and time functions
pub fn create_dates_module() -> Value {
    let mut module = HashMap::new();

    // Current date/time functions
    module.insert("now".to_string(), create_builtin_function("now", 0));
    module.insert("utc_now".to_string(), create_builtin_function("utc_now", 0));
    module.insert("today".to_string(), create_builtin_function("today", 0));

    // Date creation functions
    module.insert("date".to_string(), create_builtin_function("date", 3));
    module.insert(
        "datetime".to_string(),
        create_builtin_function("datetime", 6),
    );
    module.insert("time".to_string(), create_builtin_function("time", 3));

    // Parsing functions
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
        fields: module,
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
        "now" | "utc_now" | "today" | "is_leap_year" | "days_in_month"
    )
}

/// Public entry point. Fallible operations (anything parsing or producing a
/// date string) are wrapped into an olang `Result`: success becomes
/// `Ok(value)` and any failure — a malformed date, an out-of-range
/// component, arithmetic overflow, or a wrong-typed argument — becomes
/// `Err(message)`, so a bad date never aborts the program. Total operations
/// pass through as bare values.
pub fn call_dates_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    if is_total(name) {
        return dispatch_dates(name, args);
    }
    match dispatch_dates(name, args) {
        Ok(value) => Ok(Value::Ok(Box::new(value))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(std::sync::Arc::new(
            e.to_string(),
        ))))),
    }
}

fn dispatch_dates(name: &str, args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "now" => dates_now(args),
        "utc_now" => dates_utc_now(args),
        "today" => dates_today(args),
        "date" => dates_date(args),
        "datetime" => dates_datetime(args),
        "time" => dates_time(args),
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
        return Err(format!("now expects 0 arguments, got {}", args.len()).into());
    }

    let now = crate::clock::local_now_fixed();
    Ok(Value::String(now.to_rfc3339().into()))
}

/// Current UTC date and time
/// Usage: dates.utc_now() -> "2024-06-15T14:30:00Z"
fn dates_utc_now(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("utc_now expects 0 arguments, got {}", args.len()).into());
    }

    let now = crate::clock::utc_now();
    Ok(Value::String(now.to_rfc3339().into()))
}

/// Current local date
/// Usage: dates.today() -> "2024-06-15"
fn dates_today(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("today expects 0 arguments, got {}", args.len()).into());
    }

    let today = crate::clock::local_now_fixed().date_naive();
    Ok(Value::String(today.to_string().into()))
}

/// Create a date from year, month, day
/// Usage: dates.date(2024, 6, 15) -> "2024-06-15"
fn dates_date(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Err(format!("date expects 3 arguments, got {}", args.len()).into());
    }

    let year = match &args[0] {
        Value::Integer(y) => *y as i32,
        _ => return Err("date: year must be an integer".into()),
    };

    let month = match &args[1] {
        Value::Integer(m) => *m as u32,
        _ => return Err("date: month must be an integer".into()),
    };

    let day = match &args[2] {
        Value::Integer(d) => *d as u32,
        _ => return Err("date: day must be an integer".into()),
    };

    match NaiveDate::from_ymd_opt(year, month, day) {
        Some(date) => Ok(Value::String(date.to_string().into())),
        None => Err(format!("Invalid date: {}-{:02}-{:02}", year, month, day).into()),
    }
}

/// Create a datetime from year, month, day, hour, minute, second
/// Usage: dates.datetime(2024, 6, 15, 14, 30, 0) -> "2024-06-15T14:30:00"
fn dates_datetime(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 6 {
        return Err(format!("datetime expects 6 arguments, got {}", args.len()).into());
    }

    let year = match &args[0] {
        Value::Integer(y) => *y as i32,
        _ => return Err("datetime: year must be an integer".into()),
    };

    let month = match &args[1] {
        Value::Integer(m) => *m as u32,
        _ => return Err("datetime: month must be an integer".into()),
    };

    let day = match &args[2] {
        Value::Integer(d) => *d as u32,
        _ => return Err("datetime: day must be an integer".into()),
    };

    let hour = match &args[3] {
        Value::Integer(h) => *h as u32,
        _ => return Err("datetime: hour must be an integer".into()),
    };

    let minute = match &args[4] {
        Value::Integer(m) => *m as u32,
        _ => return Err("datetime: minute must be an integer".into()),
    };

    let second = match &args[5] {
        Value::Integer(s) => *s as u32,
        _ => return Err("datetime: second must be an integer".into()),
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
        return Err(format!("time expects 3 arguments, got {}", args.len()).into());
    }

    let hour = match &args[0] {
        Value::Integer(h) => *h as u32,
        _ => return Err("time: hour must be an integer".into()),
    };

    let minute = match &args[1] {
        Value::Integer(m) => *m as u32,
        _ => return Err("time: minute must be an integer".into()),
    };

    let second = match &args[2] {
        Value::Integer(s) => *s as u32,
        _ => return Err("time: second must be an integer".into()),
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
        return Err(format!("parse_date expects 1 argument, got {}", args.len()).into());
    }

    let date_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("parse_date: argument must be a string".into()),
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
        return Err(format!("parse_datetime expects 1 argument, got {}", args.len()).into());
    }

    let datetime_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("parse_datetime: argument must be a string".into()),
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
        return Err(format!("parse_time expects 1 argument, got {}", args.len()).into());
    }

    let time_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("parse_time: argument must be a string".into()),
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
        return Err(format!("format_date expects 2 arguments, got {}", args.len()).into());
    }

    let date_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("format_date: first argument must be a string".into()),
    };

    let format_str = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => return Err("format_date: second argument must be a string".into()),
    };

    let date = match NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => {
            return Err(format!(
                "Cannot parse date: '{}'. Expected format: YYYY-MM-DD",
                date_str
            )
            .into());
        }
    };

    let formatted = date.format(format_str).to_string();
    Ok(Value::String(formatted.into()))
}

/// Format a datetime with a custom format string
/// Usage: dates.format_datetime("2024-06-15T14:30:00", "%B %d, %Y at %I:%M %p") -> "June 15, 2024 at 02:30 PM"
fn dates_format_datetime(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("format_datetime expects 2 arguments, got {}", args.len()).into());
    }

    let datetime_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("format_datetime: first argument must be a string".into()),
    };

    let format_str = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => return Err("format_datetime: second argument must be a string".into()),
    };

    let datetime = parse_datetime_flexible(datetime_str)?;

    let formatted = datetime.format(format_str).to_string();
    Ok(Value::String(formatted.into()))
}

/// Format a time with a custom format string
/// Usage: dates.format_time("14:30:00", "%I:%M %p") -> "02:30 PM"
fn dates_format_time(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("format_time expects 2 arguments, got {}", args.len()).into());
    }

    let time_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("format_time: first argument must be a string".into()),
    };

    let format_str = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => return Err("format_time: second argument must be a string".into()),
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
        return Err(format!("add_days expects 2 arguments, got {}", args.len()).into());
    }

    let date_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("add_days: first argument must be a string".into()),
    };

    let days = match &args[1] {
        Value::Integer(d) => *d,
        _ => return Err("add_days: second argument must be an integer".into()),
    };

    let date = match NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => {
            return Err(format!(
                "Cannot parse date: '{}'. Expected format: YYYY-MM-DD",
                date_str
            )
            .into());
        }
    };

    let result = if days < 0 {
        date.checked_sub_days(chrono::Days::new(days.unsigned_abs()))
    } else {
        date.checked_add_days(chrono::Days::new(days as u64))
    };

    match result {
        Some(new_date) => Ok(Value::String(new_date.to_string().into())),
        None => Err("Date arithmetic overflow".into()),
    }
}

/// Add weeks to a date
/// Usage: dates.add_weeks("2024-06-15", 2) -> "2024-06-29"
fn dates_add_weeks(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("add_weeks expects 2 arguments, got {}", args.len()).into());
    }

    let weeks = match &args[1] {
        Value::Integer(w) => w
            .checked_mul(7) // Convert weeks to days
            .ok_or("add_weeks: overflow converting weeks to days")?,
        _ => return Err("add_weeks: second argument must be an integer".into()),
    };

    // Reuse add_days logic
    dates_add_days(vec![args[0].clone(), Value::Integer(weeks)])
}

/// Add months to a date (approximately)
/// Usage: dates.add_months("2024-06-15", 3) -> "2024-09-15"
fn dates_add_months(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("add_months expects 2 arguments, got {}", args.len()).into());
    }

    let date_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("add_months: first argument must be a string".into()),
    };

    let months = match &args[1] {
        Value::Integer(m) => *m,
        _ => return Err("add_months: second argument must be an integer".into()),
    };

    let date = match NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => {
            return Err(format!(
                "Cannot parse date: '{}'. Expected format: YYYY-MM-DD",
                date_str
            )
            .into());
        }
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
        Some(new_date) => Ok(Value::String(new_date.to_string().into())),
        None => {
            // Handle cases where the day doesn't exist in the target month (e.g., Jan 31 + 1 month)
            let last_day_of_month = NaiveDate::from_ymd_opt(year, month as u32, 1)
                .and_then(|d| d.checked_add_months(chrono::Months::new(1)))
                .and_then(|d| d.checked_sub_days(chrono::Days::new(1)))
                .map(|d| d.day())
                .unwrap_or(28);

            let adjusted_day = day.min(last_day_of_month);
            match NaiveDate::from_ymd_opt(year, month as u32, adjusted_day) {
                Some(new_date) => Ok(Value::String(new_date.to_string().into())),
                None => Err("Date arithmetic error".into()),
            }
        }
    }
}

/// Add years to a date
/// Usage: dates.add_years("2024-06-15", 1) -> "2025-06-15"
fn dates_add_years(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("add_years expects 2 arguments, got {}", args.len()).into());
    }

    let years = match &args[1] {
        Value::Integer(y) => y
            .checked_mul(12) // Convert years to months
            .ok_or("add_years: overflow converting years to months")?,
        _ => return Err("add_years: second argument must be an integer".into()),
    };

    // Reuse add_months logic
    dates_add_months(vec![args[0].clone(), Value::Integer(years)])
}

/// Calculate difference in days between two dates
/// Usage: dates.diff_days("2024-06-22", "2024-06-15") -> 7
fn dates_diff_days(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("diff_days expects 2 arguments, got {}", args.len()).into());
    }

    let date1_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("diff_days: first argument must be a string".into()),
    };

    let date2_str = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => return Err("diff_days: second argument must be a string".into()),
    };

    let date1 = match NaiveDate::parse_from_str(date1_str, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => {
            return Err(format!(
                "Cannot parse first date: '{}'. Expected format: YYYY-MM-DD",
                date1_str
            )
            .into());
        }
    };

    let date2 = match NaiveDate::parse_from_str(date2_str, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => {
            return Err(format!(
                "Cannot parse second date: '{}'. Expected format: YYYY-MM-DD",
                date2_str
            )
            .into());
        }
    };

    let diff = date1.signed_duration_since(date2);
    Ok(Value::Integer(diff.num_days()))
}

/// Extract year from date
/// Usage: dates.year("2024-06-15") -> 2024
fn dates_year(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("year expects 1 argument, got {}", args.len()).into());
    }

    let date_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("year: argument must be a string".into()),
    };

    let date = parse_date_flexible(date_str)?;

    Ok(Value::Integer(date.year() as i64))
}

/// Extract month from date
/// Usage: dates.month("2024-06-15") -> 6
fn dates_month(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("month expects 1 argument, got {}", args.len()).into());
    }

    let date_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("month: argument must be a string".into()),
    };

    let date = parse_date_flexible(date_str)?;

    Ok(Value::Integer(date.month() as i64))
}

/// Extract day from date
/// Usage: dates.day("2024-06-15") -> 15
fn dates_day(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("day expects 1 argument, got {}", args.len()).into());
    }

    let date_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("day: argument must be a string".into()),
    };

    let date = parse_date_flexible(date_str)?;

    Ok(Value::Integer(date.day() as i64))
}

/// Extract hour from datetime
/// Usage: dates.hour("2024-06-15T14:30:00") -> 14
fn dates_hour(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("hour expects 1 argument, got {}", args.len()).into());
    }

    let datetime_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("hour: argument must be a string".into()),
    };

    let datetime = parse_datetime_flexible(datetime_str)?;

    Ok(Value::Integer(datetime.hour() as i64))
}

/// Extract minute from datetime
/// Usage: dates.minute("2024-06-15T14:30:00") -> 30
fn dates_minute(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("minute expects 1 argument, got {}", args.len()).into());
    }

    let datetime_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("minute: argument must be a string".into()),
    };

    let datetime = parse_datetime_flexible(datetime_str)?;

    Ok(Value::Integer(datetime.minute() as i64))
}

/// Extract second from datetime
/// Usage: dates.second("2024-06-15T14:30:45") -> 45
fn dates_second(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("second expects 1 argument, got {}", args.len()).into());
    }

    let datetime_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("second: argument must be a string".into()),
    };

    let datetime = parse_datetime_flexible(datetime_str)?;

    Ok(Value::Integer(datetime.second() as i64))
}

/// Get weekday from date (0=Sunday, 1=Monday, ..., 6=Saturday)
/// Usage: dates.weekday("2024-06-15") -> 6 (Saturday)
fn dates_weekday(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("weekday expects 1 argument, got {}", args.len()).into());
    }

    let date_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("weekday: argument must be a string".into()),
    };

    let date = parse_date_flexible(date_str)?;

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
        return Err(format!("is_leap_year expects 1 argument, got {}", args.len()).into());
    }

    let year = match &args[0] {
        Value::Integer(y) => *y as i32,
        _ => return Err("is_leap_year: argument must be an integer".into()),
    };

    let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    Ok(Value::Boolean(is_leap))
}

/// Get number of days in a month
/// Usage: dates.days_in_month(2024, 2) -> 29
fn dates_days_in_month(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("days_in_month expects 2 arguments, got {}", args.len()).into());
    }

    let year = match &args[0] {
        Value::Integer(y) => *y as i32,
        _ => return Err("days_in_month: first argument (year) must be an integer".into()),
    };

    let month = match &args[1] {
        Value::Integer(m) => *m as u32,
        _ => return Err("days_in_month: second argument (month) must be an integer".into()),
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
        return Err(format!("timestamp expects 1 argument, got {}", args.len()).into());
    }

    let datetime_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => return Err("timestamp: argument must be a string".into()),
    };

    let datetime = parse_datetime_flexible(datetime_str)?;

    let timestamp = datetime.and_utc().timestamp();
    Ok(Value::Integer(timestamp))
}

/// Convert Unix timestamp to datetime
/// Usage: dates.from_timestamp(1718461800) -> "2024-06-15T14:30:00"
fn dates_from_timestamp(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("from_timestamp expects 1 argument, got {}", args.len()).into());
    }

    let timestamp = match &args[0] {
        Value::Integer(ts) => *ts,
        _ => return Err("from_timestamp: argument must be an integer".into()),
    };

    match DateTime::from_timestamp(timestamp, 0) {
        Some(datetime) => Ok(Value::String(datetime.naive_utc().to_string().into())),
        None => Err(format!("Invalid timestamp: {}", timestamp).into()),
    }
}
