//! Request logging that cannot identify anyone, and the one log that deliberately can.
//!
//! A request line holds the method, the route *pattern* (`/v1/channels/{channel}`,
//! never the real path), the status and the time taken. No device IDs, channel IDs,
//! names, addresses, tokens or query strings. Successful queue traffic is not logged at
//! all, so the log does not even record when messages moved.
//!
//! The exception is a wrong server password: that is logged with the address it came
//! from, so a firewall tool can ban whoever is guessing ([`password_line`]). Nobody who
//! knows the password, and nobody using any other way in, ever appears.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{MatchedPath, Request};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

const QUIET_WHEN_OK: [&str; 4] = ["/v1/channels/{channel}/messages", "/v1/queue", "/v1/queue/ack", "/v1/events"];

pub fn line(method: &Method, route: Option<&str>, status: StatusCode, elapsed: Duration) -> Option<String> {
    let route = route.unwrap_or("(no such route)");
    if QUIET_WHEN_OK.contains(&route) && !status.is_client_error() && !status.is_server_error() {
        return None;
    }
    Some(format!("{method} {route} {} {}ms", status.as_u16(), elapsed.as_millis()))
}

/// What happened to an attempt at the server password.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordEvent {
    /// The password was wrong.
    Failure,
    /// The attempt was turned away unchecked: too many wrong passwords from this address.
    Refused,
}

/// One line for a firewall tool to read, e.g.
/// `2026-11-26T18:20:05Z password-failure from=203.0.113.7`.
/// The time is UTC. The address is the caller's, or `unknown`.
pub fn password_line(unix_time: i64, event: PasswordEvent, address: Option<IpAddr>) -> String {
    let event = match event {
        PasswordEvent::Failure => "failure",
        PasswordEvent::Refused => "refused",
    };
    let from = address.map_or("unknown".to_owned(), |address| address.to_canonical().to_string());
    format!("{} password-{event} from={from}", utc(unix_time))
}

/// Unix seconds as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn utc(unix_time: i64) -> String {
    let (days, seconds) = (unix_time.div_euclid(86_400), unix_time.rem_euclid(86_400));
    // Days since 1970 to a calendar date ("civil from days", Howard Hinnant).
    let shifted = days + 719_468;
    let (era, day_of_era) = (shifted.div_euclid(146_097), shifted.rem_euclid(146_097));
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let (hour, minute, second) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Where wrong-password lines go: standard output, or a list in memory for tests.
#[derive(Clone, Default)]
pub struct FailureLog {
    captured: Arc<Mutex<Option<Vec<String>>>>,
}

impl FailureLog {
    pub fn write(&self, line: String) {
        match self.captured.lock().unwrap().as_mut() {
            Some(lines) => lines.push(line),
            None => println!("{line}"),
        }
    }

    /// From now on, keep lines in memory instead of printing them.
    pub fn capture(&self) {
        self.captured.lock().unwrap().get_or_insert_default();
    }

    /// The lines kept since [`FailureLog::capture`].
    pub fn captured(&self) -> Vec<String> {
        self.captured.lock().unwrap().clone().unwrap_or_default()
    }
}

pub async fn log(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let route = request.extensions().get::<MatchedPath>().map(|path| path.as_str().to_owned());
    let started = Instant::now();
    let response = next.run(request).await;
    if let Some(line) = line(&method, route.as_deref(), response.status(), started.elapsed()) {
        println!("{line}");
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_hold_the_pattern_and_nothing_else() {
        let line = line(&Method::GET, Some("/v1/channels/{channel}"), StatusCode::NOT_FOUND, Duration::from_millis(3));
        assert_eq!(line.as_deref(), Some("GET /v1/channels/{channel} 404 3ms"));
    }

    #[test]
    fn password_lines_carry_a_time_and_the_callers_address() {
        let v4 = "203.0.113.7".parse().ok();
        assert_eq!(
            password_line(1_800_000_000, PasswordEvent::Failure, v4),
            "2027-01-15T08:00:00Z password-failure from=203.0.113.7"
        );
        let v6 = "2001:db8::5".parse().ok();
        assert_eq!(
            password_line(1_800_000_000, PasswordEvent::Refused, v6),
            "2027-01-15T08:00:00Z password-refused from=2001:db8::5"
        );
        // An IPv4 caller reaching an IPv6 socket is written the way a firewall expects.
        let mapped = "::ffff:203.0.113.7".parse().ok();
        assert!(password_line(0, PasswordEvent::Failure, mapped).ends_with("from=203.0.113.7"));
        assert!(password_line(0, PasswordEvent::Failure, None).ends_with("from=unknown"));
    }

    #[test]
    fn utc_times_match_the_calendar() {
        // Expected values from Python's datetime module.
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(951_782_399), "2000-02-28T23:59:59Z");
        assert_eq!(utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc(1_709_210_096), "2024-02-29T12:34:56Z");
        assert_eq!(utc(1_735_689_599), "2024-12-31T23:59:59Z");
        assert_eq!(utc(4_102_444_800), "2100-01-01T00:00:00Z");
    }

    #[test]
    fn unknown_paths_are_not_echoed() {
        let line = line(&Method::GET, None, StatusCode::NOT_FOUND, Duration::ZERO);
        assert_eq!(line.as_deref(), Some("GET (no such route) 404 0ms"));
    }

    #[test]
    fn successful_queue_traffic_leaves_no_trace() {
        for route in QUIET_WHEN_OK {
            assert_eq!(line(&Method::POST, Some(route), StatusCode::CREATED, Duration::ZERO), None);
            assert!(line(&Method::POST, Some(route), StatusCode::FORBIDDEN, Duration::ZERO).is_some());
        }
    }
}
