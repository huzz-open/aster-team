//! Local-calendar price selection. The caller resolves UTC to the configured
//! pricing time zone before selecting a rate; member device time is irrelevant.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::Date;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriceWindow<T> {
    /// Inclusive local date on which the window starts. `None` is unbounded.
    pub start_date: Option<Date>,
    /// Exclusive local date on which the window starts. `None` is unbounded.
    pub end_date: Option<Date>,
    /// Bit 0 is Monday; bit 6 is Sunday. The weekday belongs to the start date.
    pub weekdays: u8,
    /// Minutes after local midnight, from 0 through 1439.
    pub start_minute: u16,
    /// Exclusive minute, from 0 through 1440. A value below `start_minute`
    /// continues into the next local date.
    pub end_minute: u16,
    pub price: T,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriceSchedule<T> {
    pub base: T,
    pub windows: Vec<PriceWindow<T>>,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ScheduleError {
    #[error("invalid price window")]
    InvalidWindow,
    #[error("price windows overlap")]
    OverlappingWindows,
    #[error("invalid local minute")]
    InvalidMinute,
}

impl<T> PriceSchedule<T> {
    pub fn validate(&self) -> Result<(), ScheduleError> {
        for window in &self.windows {
            window.validate()?;
        }
        for (index, first) in self.windows.iter().enumerate() {
            for second in self.windows.iter().skip(index + 1) {
                if windows_overlap(first, second) {
                    return Err(ScheduleError::OverlappingWindows);
                }
            }
        }
        Ok(())
    }

    pub fn price_at(&self, local_date: Date, local_minute: u16) -> Result<&T, ScheduleError> {
        if local_minute >= 1440 {
            return Err(ScheduleError::InvalidMinute);
        }
        let mut selected = None;
        for window in &self.windows {
            if window.matches(local_date, local_minute) {
                if selected.is_some() {
                    return Err(ScheduleError::OverlappingWindows);
                }
                selected = Some(&window.price);
            }
        }
        Ok(selected.unwrap_or(&self.base))
    }
}

impl<T> PriceWindow<T> {
    fn validate(&self) -> Result<(), ScheduleError> {
        if self.weekdays == 0
            || self.weekdays & !0x7f != 0
            || self.start_minute >= 1440
            || self.end_minute > 1440
            || self.start_minute == self.end_minute
            || self
                .start_date
                .zip(self.end_date)
                .is_some_and(|(start, end)| start >= end)
        {
            return Err(ScheduleError::InvalidWindow);
        }
        Ok(())
    }

    fn matches(&self, date: Date, minute: u16) -> bool {
        let crosses_midnight = self.end_minute < self.start_minute;
        let origin = if crosses_midnight && minute < self.end_minute {
            date.previous_day()
        } else if minute >= self.start_minute && (crosses_midnight || minute < self.end_minute) {
            Some(date)
        } else {
            None
        };
        let Some(origin) = origin else {
            return false;
        };
        self.start_date.is_none_or(|start| origin >= start)
            && self.end_date.is_none_or(|end| origin < end)
            && self.weekdays & (1 << origin.weekday().number_days_from_monday()) != 0
    }

    fn segments(&self) -> [Option<Segment>; 2] {
        if self.end_minute > self.start_minute {
            [
                Some(Segment {
                    day_offset: 0,
                    start: self.start_minute,
                    end: self.end_minute,
                }),
                None,
            ]
        } else {
            [
                Some(Segment {
                    day_offset: 0,
                    start: self.start_minute,
                    end: 1440,
                }),
                (self.end_minute > 0).then_some(Segment {
                    day_offset: 1,
                    start: 0,
                    end: self.end_minute,
                }),
            ]
        }
    }
}

#[derive(Clone, Copy)]
struct Segment {
    day_offset: i32,
    start: u16,
    end: u16,
}

fn windows_overlap<T>(first: &PriceWindow<T>, second: &PriceWindow<T>) -> bool {
    for a in first.segments().into_iter().flatten() {
        for b in second.segments().into_iter().flatten() {
            if a.start.max(b.start) >= a.end.min(b.end) {
                continue;
            }
            let lower = max_bound(
                first
                    .start_date
                    .map(|date| date.to_julian_day() + a.day_offset),
                second
                    .start_date
                    .map(|date| date.to_julian_day() + b.day_offset),
            );
            let upper = min_bound(
                first
                    .end_date
                    .map(|date| date.to_julian_day() + a.day_offset),
                second
                    .end_date
                    .map(|date| date.to_julian_day() + b.day_offset),
            );
            if lower.zip(upper).is_some_and(|(start, end)| start >= end) {
                continue;
            }
            // The weekday pattern repeats every seven days, so checking one
            // complete week within the common date range is sufficient.
            let anchor = lower.unwrap_or_else(|| {
                upper.map_or_else(
                    || time::macros::date!(2024 - 01 - 01).to_julian_day(),
                    |end| end - 7,
                )
            });
            for day in anchor..anchor + 7 {
                if upper.is_some_and(|end| day >= end) {
                    break;
                }
                let weekday_a = weekday_for_julian(day - a.day_offset);
                let weekday_b = weekday_for_julian(day - b.day_offset);
                if first.weekdays & (1 << weekday_a) != 0 && second.weekdays & (1 << weekday_b) != 0
                {
                    return true;
                }
            }
        }
    }
    false
}

fn weekday_for_julian(day: i32) -> u8 {
    let monday = time::macros::date!(2024 - 01 - 01).to_julian_day();
    (day - monday).rem_euclid(7) as u8
}

fn max_bound(first: Option<i32>, second: Option<i32>) -> Option<i32> {
    match (first, second) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

fn min_bound(first: Option<i32>, second: Option<i32>) -> Option<i32> {
    match (first, second) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    fn weekday_window(start: u16, end: u16, weekdays: u8, price: u8) -> PriceWindow<u8> {
        PriceWindow {
            start_date: None,
            end_date: None,
            weekdays,
            start_minute: start,
            end_minute: end,
            price,
        }
    }

    #[test]
    fn empty_schedule_uses_base_at_every_time() {
        let schedule = PriceSchedule {
            base: 7,
            windows: Vec::<PriceWindow<u8>>::new(),
        };
        schedule.validate().unwrap();
        assert_eq!(*schedule.price_at(date!(2024 - 01 - 01), 0).unwrap(), 7);
        assert_eq!(*schedule.price_at(date!(2024 - 01 - 01), 1439).unwrap(), 7);
    }

    #[test]
    fn crossing_midnight_belongs_to_its_starting_weekday_and_date() {
        let monday = 1;
        let schedule = PriceSchedule {
            base: 7,
            windows: vec![PriceWindow {
                start_date: Some(date!(2024 - 01 - 01)),
                end_date: Some(date!(2024 - 01 - 02)),
                ..weekday_window(22 * 60, 2 * 60, monday, 3)
            }],
        };
        schedule.validate().unwrap();
        assert_eq!(
            *schedule.price_at(date!(2024 - 01 - 01), 22 * 60).unwrap(),
            3
        );
        assert_eq!(*schedule.price_at(date!(2024 - 01 - 02), 119).unwrap(), 3);
        assert_eq!(*schedule.price_at(date!(2024 - 01 - 02), 120).unwrap(), 7);
        assert_eq!(
            *schedule.price_at(date!(2024 - 01 - 08), 22 * 60).unwrap(),
            7
        );
    }

    #[test]
    fn overlapping_windows_are_rejected_even_across_midnight() {
        let schedule = PriceSchedule {
            base: 7,
            windows: vec![
                weekday_window(22 * 60, 2 * 60, 1, 3),
                weekday_window(60, 180, 1 << 1, 4),
            ],
        };
        assert_eq!(schedule.validate(), Err(ScheduleError::OverlappingWindows));
    }

    #[test]
    fn adjacent_and_date_disjoint_windows_are_allowed() {
        let first = weekday_window(60, 120, 1, 3);
        let second = weekday_window(120, 180, 1, 4);
        let schedule = PriceSchedule {
            base: 7,
            windows: vec![first, second],
        };
        schedule.validate().unwrap();
        let one_day = PriceWindow {
            start_date: Some(date!(2024 - 01 - 01)),
            end_date: Some(date!(2024 - 01 - 02)),
            ..weekday_window(60, 120, 1, 3)
        };
        let next_week = PriceWindow {
            start_date: Some(date!(2024 - 01 - 08)),
            end_date: Some(date!(2024 - 01 - 09)),
            ..weekday_window(60, 120, 1, 4)
        };
        PriceSchedule {
            base: 7,
            windows: vec![one_day, next_week],
        }
        .validate()
        .unwrap();
    }
}
