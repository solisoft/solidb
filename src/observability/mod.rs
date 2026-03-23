pub mod propagation {
    use axum::http::header::HeaderValue;
    use axum::http::Request;

    pub const TRACEPARENT_HEADER: &str = "traceparent";
    pub const TRACESTATE_HEADER: &str = "tracestate";

    #[derive(Debug, Clone)]
    pub struct TraceContext {
        pub version: String,
        pub trace_id: String,
        pub span_id: String,
        pub trace_flags: String,
    }

    impl TraceContext {
        pub fn new(trace_id: String, span_id: String) -> Self {
            Self {
                version: "00".to_string(),
                trace_id,
                span_id,
                trace_flags: "01".to_string(),
            }
        }

        pub fn to_header(&self) -> String {
            format!(
                "{}-{}-{}-{}",
                self.version, self.trace_id, self.span_id, self.trace_flags
            )
        }

        pub fn from_header(header: &str) -> Option<Self> {
            let parts: Vec<&str> = header.split('-').collect();
            if parts.len() >= 4 {
                return Some(TraceContext {
                    version: parts[0].to_string(),
                    trace_id: parts[1].to_string(),
                    span_id: parts[2].to_string(),
                    trace_flags: parts[3].to_string(),
                });
            }
            None
        }
    }

    pub fn extract_trace_context<B>(request: &Request<B>) -> Option<TraceContext> {
        request
            .headers()
            .get(TRACEPARENT_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(TraceContext::from_header)
    }

    pub fn inject_trace_context<B>(
        mut request: Request<B>,
        trace_ctx: &TraceContext,
    ) -> Request<B> {
        if let Ok(header_value) = HeaderValue::from_str(&trace_ctx.to_header()) {
            request
                .headers_mut()
                .insert(TRACEPARENT_HEADER, header_value);
        }
        request
    }
}

pub mod metrics {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    #[derive(Debug, Clone)]
    pub struct RequestMetrics {
        pub requests_total: Arc<AtomicU64>,
        pub requests_in_flight: Arc<AtomicU64>,
        pub latency_sum: Arc<AtomicU64>,
        pub latency_count: Arc<AtomicU64>,
    }

    impl RequestMetrics {
        pub fn new() -> Self {
            Self {
                requests_total: Arc::new(AtomicU64::new(0)),
                requests_in_flight: Arc::new(AtomicU64::new(0)),
                latency_sum: Arc::new(AtomicU64::new(0)),
                latency_count: Arc::new(AtomicU64::new(0)),
            }
        }

        pub fn record_request(&self) {
            self.requests_total.fetch_add(1, Ordering::Relaxed);
            self.requests_in_flight.fetch_add(1, Ordering::Relaxed);
        }

        pub fn end_request(&self, latency_ms: u64) {
            self.requests_in_flight.fetch_sub(1, Ordering::Relaxed);
            self.latency_sum.fetch_add(latency_ms, Ordering::Relaxed);
            self.latency_count.fetch_add(1, Ordering::Relaxed);
        }

        pub fn get_stats(&self) -> (u64, u64, u64, u64) {
            (
                self.requests_total.load(Ordering::Relaxed),
                self.requests_in_flight.load(Ordering::Relaxed),
                self.latency_sum.load(Ordering::Relaxed),
                self.latency_count.load(Ordering::Relaxed),
            )
        }
    }

    impl Default for RequestMetrics {
        fn default() -> Self {
            Self::new()
        }
    }
}

pub fn get_current_trace_context() -> Option<propagation::TraceContext> {
    use opentelemetry::trace::TraceContextExt;

    let span = tracing::Span::current();
    let ctx = span.context();

    use tracing_opentelemetry::OpenTelemetrySpanExt;
    let otel_ctx = ctx.clone();

    let span_ref = otel_ctx.span();
    let span_ctx = span_ref.span_context();
    if span_ctx.is_valid() {
        return Some(propagation::TraceContext::new(
            format!("{}", span_ctx.trace_id()),
            format!("{}", span_ctx.span_id()),
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::propagation::*;

    #[test]
    fn test_trace_context_roundtrip() {
        let ctx = TraceContext::new(
            "0af7651916cd43dd8448eb211c80319c".to_string(),
            "b7ad6b7169203331".to_string(),
        );
        let header = ctx.to_header();
        assert_eq!(
            header,
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
        );

        let ctx2 = TraceContext::from_header(&header).unwrap();
        assert_eq!(ctx.trace_id, ctx2.trace_id);
        assert_eq!(ctx.span_id, ctx2.span_id);
    }
}
