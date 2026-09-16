//! The thinnest possible HTTP client, behind a trait so the fetchers can be
//! tested against canned responses.

use super::UsageError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: String,
    /// The `Retry-After` header, when the server sent one.
    pub retry_after: Option<String>,
}

pub trait Http {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response, UsageError>;
}

/// A real client. Every request has a timeout: a hung fetch in a menu bar
/// app is worse than a failed one.
pub struct UreqHttp {
    pub timeout_seconds: u64,
}

impl Default for UreqHttp {
    fn default() -> Self {
        UreqHttp {
            timeout_seconds: 30,
        }
    }
}

impl Http for UreqHttp {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response, UsageError> {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(self.timeout_seconds))
            .build();
        let mut request = agent.get(url);
        for (name, value) in headers {
            request = request.set(name, value);
        }
        let response = match request.call() {
            Ok(response) => response,
            // A non-2xx status is still an answer worth reading.
            Err(ureq::Error::Status(_, response)) => response,
            Err(ureq::Error::Transport(e)) => return Err(UsageError::Network(e.to_string())),
        };
        let status = response.status();
        let retry_after = response.header("Retry-After").map(str::to_string);
        let body = response
            .into_string()
            .map_err(|e| UsageError::Network(e.to_string()))?;
        Ok(Response {
            status,
            body,
            retry_after,
        })
    }
}

/// A request as the fake saw it: the URL and the headers sent.
pub type RecordedRequest = (String, Vec<(String, String)>);

/// A canned client for tests: answers by URL, records what was asked.
#[derive(Default)]
pub struct FakeHttp {
    pub responses: std::cell::RefCell<Vec<(String, Response)>>,
    pub requests: std::cell::RefCell<Vec<RecordedRequest>>,
}

impl FakeHttp {
    pub fn new() -> FakeHttp {
        FakeHttp::default()
    }

    /// Answer any URL containing `fragment` with `status` and `body`.
    pub fn respond(&self, fragment: &str, status: u16, body: &str) {
        self.responses.borrow_mut().push((
            fragment.to_string(),
            Response {
                status,
                body: body.to_string(),
                retry_after: None,
            },
        ));
    }

    /// The headers sent with the first request whose URL contains `fragment`.
    pub fn headers_sent(&self, fragment: &str) -> Vec<(String, String)> {
        self.requests
            .borrow()
            .iter()
            .find(|(url, _)| url.contains(fragment))
            .map(|(_, headers)| headers.clone())
            .unwrap_or_default()
    }
}

impl Http for FakeHttp {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response, UsageError> {
        self.requests.borrow_mut().push((
            url.to_string(),
            headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ));
        self.responses
            .borrow()
            .iter()
            .find(|(fragment, _)| url.contains(fragment.as_str()))
            .map(|(_, response)| response.clone())
            .ok_or_else(|| UsageError::Network(format!("no canned response for {url}")))
    }
}

/// Seconds to wait, from a `Retry-After` header.
pub fn retry_after_seconds(header: Option<&str>) -> Option<u64> {
    let text = header?.trim();
    if let Ok(seconds) = text.parse::<u64>() {
        return Some(seconds);
    }
    let when = chrono::DateTime::parse_from_rfc2822(text).ok()?;
    let delta = when.timestamp() - chrono::Utc::now().timestamp();
    Some(delta.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_accepts_seconds_and_dates() {
        assert_eq!(retry_after_seconds(Some("120")), Some(120));
        assert_eq!(retry_after_seconds(Some(" 5 ")), Some(5));
        assert_eq!(retry_after_seconds(Some("Wed, 21 Oct 2015 07:28:00 GMT")), Some(0));
        assert_eq!(retry_after_seconds(Some("garbage")), None);
        assert_eq!(retry_after_seconds(None), None);
    }

    #[test]
    fn the_fake_answers_by_fragment_and_records_headers() {
        let http = FakeHttp::new();
        http.respond("/usage", 200, "{}");
        let response = http.get("https://x/api/usage", &[("Authorization", "Bearer t")]).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(http.headers_sent("/usage"), vec![("Authorization".to_string(), "Bearer t".to_string())]);
        assert!(http.get("https://x/other", &[]).is_err());
    }
}
