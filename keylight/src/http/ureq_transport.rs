use super::{HttpResponse, Transport, TransportOutcome};

pub struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(15))
                .build(),
        }
    }
}

fn handle(res: Result<ureq::Response, ureq::Error>) -> TransportOutcome {
    match res {
        Ok(resp) => {
            let status = resp.status();
            let retry_after = resp.header("Retry-After").and_then(|h| h.parse::<u64>().ok());
            let body = resp.into_string().unwrap_or_default();
            TransportOutcome::Response(HttpResponse { status, body, retry_after })
        }
        Err(ureq::Error::Status(_, resp)) => {
            let status = resp.status();
            let retry_after = resp.header("Retry-After").and_then(|h| h.parse::<u64>().ok());
            let body = resp.into_string().unwrap_or_default();
            TransportOutcome::Response(HttpResponse { status, body, retry_after })
        }
        Err(ureq::Error::Transport(t)) => {
            use ureq::ErrorKind::*;
            match t.kind() {
                Io | Dns | ConnectionFailed => TransportOutcome::Transient(t.to_string()),
                _ => TransportOutcome::Terminal(t.to_string()),
            }
        }
    }
}

impl Transport for UreqTransport {
    fn post_json(&self, url: &str, headers: &[(String, String)], body: &str) -> TransportOutcome {
        let mut req = self.agent.post(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        handle(req.send_string(body))
    }

    fn get(&self, url: &str, headers: &[(String, String)]) -> TransportOutcome {
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        handle(req.call())
    }
}
