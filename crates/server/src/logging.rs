//! Request logging that cannot identify anyone.
//!
//! A line holds the method, the route *pattern* (`/v1/channels/{channel}`, never the
//! real path), the status and the time taken. No device IDs, channel IDs, names,
//! addresses, tokens or query strings. Successful queue traffic is not logged at all,
//! so the log does not even record when messages moved.

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
