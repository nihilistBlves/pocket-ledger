//! Calendar logic of the dashboard. Pure functions: "today" is a parameter.

use chrono::{Datelike, Months, NaiveDate};

use super::{LedgerError, Result};

/// The selected month and its comparison period, as inclusive date ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Periods {
    pub month_start: NaiveDate,
    /// Last day of the calendar month.
    pub month_end: NaiveDate,
    pub is_current: bool,
    /// Day of month the figures run to; `None` for past (or future) months.
    pub through_day: Option<u32>,
    /// Range of the totals: the whole month, or up to today for the current one.
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub prev_from: NaiveDate,
    pub prev_to: NaiveDate,
}

pub fn parse_month(month: &str) -> Result<NaiveDate> {
    let bytes = month.as_bytes();
    let shaped = bytes.len() == 7
        && bytes[4] == b'-'
        && month.chars().filter(char::is_ascii_digit).count() == 6;
    if !shaped {
        return Err(LedgerError::InvalidInput("month must be YYYY-MM"));
    }
    NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d")
        .map_err(|_| LedgerError::InvalidInput("month must be YYYY-MM"))
}

pub fn parse_date(date: &str) -> Result<NaiveDate> {
    let bytes = date.as_bytes();
    let shaped = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && date.chars().filter(char::is_ascii_digit).count() == 8;
    if !shaped {
        return Err(LedgerError::InvalidInput("date must be YYYY-MM-DD"));
    }
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| LedgerError::InvalidInput("date must be YYYY-MM-DD"))
}

/// Last day of the month that starts at `first` (which must be day 1).
pub fn last_day_of_month(first: NaiveDate) -> NaiveDate {
    first
        .checked_add_months(Months::new(1))
        .and_then(|d| d.pred_opt())
        .unwrap_or(first)
}

/// Selected month and comparison period.
///
/// Current month (the month of `today`): day 1..today against day 1..same day of the
/// previous month, clamped to that month's length (31 Mar -> 28/29 Feb). Any other
/// month: the full month against the full previous month.
pub fn periods(month: &str, today: &str) -> Result<Periods> {
    let month_start = parse_month(month)?;
    let today = parse_date(today)?;
    let month_end = last_day_of_month(month_start);
    let prev_start = month_start
        .checked_sub_months(Months::new(1))
        .ok_or(LedgerError::InvalidInput("month out of range"))?;
    let prev_end = last_day_of_month(prev_start);
    let is_current = today.year() == month_start.year() && today.month() == month_start.month();
    if is_current {
        let day = today.day();
        let prev_to = prev_start.with_day(day.min(prev_end.day())).unwrap_or(prev_end);
        Ok(Periods {
            month_start,
            month_end,
            is_current,
            through_day: Some(day),
            from: month_start,
            to: today,
            prev_from: prev_start,
            prev_to,
        })
    } else {
        Ok(Periods {
            month_start,
            month_end,
            is_current,
            through_day: None,
            from: month_start,
            to: month_end,
            prev_from: prev_start,
            prev_to: prev_end,
        })
    }
}

/// The 12 months ending at `last` (first day of each), oldest first.
pub fn twelve_months(last: NaiveDate) -> Vec<NaiveDate> {
    (0..12u32)
        .rev()
        .filter_map(|back| last.checked_sub_months(Months::new(back)))
        .collect()
}

pub fn iso(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub fn month_key(d: NaiveDate) -> String {
    d.format("%Y-%m").to_string()
}
