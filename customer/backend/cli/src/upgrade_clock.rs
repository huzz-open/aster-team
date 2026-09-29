//! Linux boot-relative deadlines survive executor restarts and wall-clock edits.
use std::{fs::File, io::Read as _};

use aster_error_catalog::delivery;
use aster_upgrade_core::runtime::{UpgradeClock, UpgradeClockSource};

use super::CliFailure;

pub(super) struct LinuxUpgradeClock;

impl UpgradeClockSource for LinuxUpgradeClock {
    type Error = CliFailure;

    fn sample() -> Result<UpgradeClock, Self::Error> {
        let boot_id = bounded_read("/proc/sys/kernel/random/boot_id")?;
        let uptime = bounded_read("/proc/uptime")?;
        parse(&boot_id, &uptime)
    }
}

fn bounded_read(path: &str) -> Result<String, CliFailure> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(257).read_to_end(&mut bytes))
        .map_err(|_| failed("cannot read Linux boot-relative upgrade clock"))?;
    if bytes.len() > 256 {
        return Err(failed("Linux upgrade clock response is too large"));
    }
    String::from_utf8(bytes).map_err(|_| failed("Linux upgrade clock response is invalid"))
}

fn parse(boot_id: &str, uptime: &str) -> Result<UpgradeClock, CliFailure> {
    let boot_id = boot_id.trim();
    if boot_id.len() != 36
        || !boot_id.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return Err(failed("Linux boot identifier is invalid"));
    }
    let uptime = uptime
        .split_whitespace()
        .next()
        .ok_or_else(|| failed("Linux uptime is empty"))?;
    let (seconds, fraction) = uptime
        .split_once('.')
        .ok_or_else(|| failed("Linux uptime has no fractional part"))?;
    if seconds.is_empty()
        || !seconds.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.is_empty()
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(failed("Linux uptime is not a nonnegative decimal"));
    }
    // Kernel uptime is normally hundredths of a second. Integer arithmetic
    // avoids floating-point loss and rejects overflow instead of saturating it.
    let milliseconds = seconds
        .parse::<u64>()
        .ok()
        .and_then(|seconds| seconds.checked_mul(1000))
        .ok_or_else(|| failed("Linux uptime overflows the upgrade clock"))?;
    let fraction_ms = fraction
        .bytes()
        .take(3)
        .chain(std::iter::repeat(b'0'))
        .take(3)
        .fold(0_u64, |value, digit| value * 10 + u64::from(digit - b'0'));
    Ok(UpgradeClock {
        boot_id: boot_id.to_owned(),
        uptime_ms: milliseconds
            .checked_add(fraction_ms)
            .ok_or_else(|| failed("Linux uptime overflows the upgrade clock"))?,
    })
}

fn failed(message: &str) -> CliFailure {
    CliFailure::new(delivery::UPGRADE_FAILED, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    const BOOT: &str = "1b52cdf8-0ad5-4e5f-89ce-8a0c0daa3089";

    #[test]
    fn boot_clock_uses_integer_uptime_without_wall_clock_or_environment_inputs() {
        for (input, expected) in [
            ("12.34 99.99\n", 12340),
            ("12.3 0.0", 12300),
            ("12.345678 0.0", 12345),
            ("0.00 0.00", 0),
        ] {
            let value = parse(&format!("{BOOT}\n"), input).unwrap();
            assert_eq!(value.boot_id, BOOT);
            assert_eq!(value.uptime_ms, expected);
        }
    }

    #[test]
    fn corrupt_negative_and_overflowing_clocks_are_rejected() {
        for input in [
            "",
            "12",
            "12.",
            "-1.0",
            "+1.0",
            "1e3.0",
            "1.NaN",
            "18446744073709552.0",
        ] {
            assert!(parse(BOOT, input).is_err(), "{input}");
        }
        assert!(parse("not-a-boot-id", "1.00").is_err());
    }
}
