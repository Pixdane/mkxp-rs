use crate::{LogError, LogTarget};
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::sync::Mutex;
use time::{OffsetDateTime, UtcOffset};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

// ---------------------------------------------------------------------------
// MkxpLayer — custom tracing subscriber layer
// ---------------------------------------------------------------------------

/// A `tracing_subscriber::Layer` that formats events in mkxp-rs style.
///
/// Output format (one event per line):
///
/// ```text
/// [2026-05-31T10:30:00.123+08:00] INFO  mkxp_audio::manager: BGM playback started path="Audio/BGM/battle.ogg"
/// ```
///
/// - Timestamp: ISO 8601 with local timezone offset.
/// - Level: right-padded to 5 characters.
/// - Target: the event's module path.
/// - Span context: when present, wrapped in `{...}` after the target.
/// - Message + structured fields: message first, then `key=value` pairs.
pub(crate) struct MkxpLayer {
    writers: Vec<Mutex<Box<dyn Write + Send>>>,
    /// Whether to log span creation (`on_new_span`) and close
    /// with duration (`on_close`).
    log_spans: bool,
}

impl MkxpLayer {
    /// Create a new layer that writes to the given `target`.
    ///
    /// `LogTarget::Composite` targets are flattened at construction time;
    /// each leaf target gets its own writer.  Nested composites are
    /// expanded recursively.
    ///
    /// For `LogTarget::File` targets, parent directories are created
    /// automatically.
    pub fn new(target: LogTarget, log_spans: bool) -> Result<Self, LogError> {
        let writers = open_writers(flatten_targets(target))?;
        Ok(MkxpLayer { writers, log_spans })
    }
}

impl<S> Layer<S> for MkxpLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut buf = String::with_capacity(256);

        // --- Timestamp (ISO 8601 with local offset) ---
        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
        write_timestamp(&mut buf, now);
        buf.push(' ');

        // --- Level (right-padded to 5 chars) ---
        let meta = event.metadata();
        let _ = write!(buf, "{:<5} ", meta.level().as_str());

        // --- Target ---
        buf.push_str(meta.target());

        // --- Span chain (innermost first) ---
        if let Some(span) = ctx.lookup_current() {
            let mut names: Vec<&str> = Vec::new();
            let mut current = Some(span);
            while let Some(s) = current {
                names.push(s.name());
                current = s.parent();
            }
            for name in names.iter().rev() {
                let _ = write!(buf, "{{{}}}", name);
            }
        }

        // --- Separator ---
        buf.push_str(": ");

        // --- Message and fields ---
        let mut visitor = EventVisitor {
            buf: &mut buf,
            first_field: true,
        };
        event.record(&mut visitor);

        // Ensure trailing newline
        if !buf.ends_with('\n') {
            buf.push('\n');
        }

        // --- Write to all targets ---
        for writer in &self.writers {
            let mut w = writer.lock().unwrap();
            let _ = w.write_all(buf.as_bytes());
        }
    }

    fn on_new_span(&self, attrs: &tracing::span::Attributes<'_>, id: &tracing::span::Id, ctx: Context<'_, S>) {
        if !self.log_spans {
            return;
        }

        // Store the creation instant in span extensions.
        let span = ctx.span(id).expect("span must exist in registry");
        span.extensions_mut().insert(std::time::Instant::now());

        // Format a log line announcing the span.
        let mut buf = String::with_capacity(128);
        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
        write_timestamp(&mut buf, now);
        buf.push(' ');

        let meta = attrs.metadata();
        let _ = write!(buf, "SPAN+ {:<5} ", meta.level().as_str());
        buf.push_str(meta.target());
        buf.push('{');
        buf.push_str(meta.name());
        buf.push('}');

        // Append parent span chain
        if let Some(parent) = ctx.lookup_current() {
            let mut names: Vec<&str> = Vec::new();
            let mut current = Some(parent);
            while let Some(s) = current {
                names.push(s.name());
                current = s.parent();
            }
            for name in names.iter().rev() {
                let _ = write!(buf, "{{{}}}", name);
            }
        }

        // Append initial field values
        let mut visitor = SpanFieldVisitor { buf: &mut buf, first: true };
        attrs.record(&mut visitor);

        buf.push('\n');

        for writer in &self.writers {
            let mut w = writer.lock().unwrap();
            let _ = w.write_all(buf.as_bytes());
        }
    }

    fn on_close(&self, id: tracing::span::Id, ctx: Context<'_, S>) {
        if !self.log_spans {
            return;
        }

        let span = ctx.span(&id).expect("span must exist in registry");
        let dur = span
            .extensions()
            .get::<std::time::Instant>()
            .map(|start| start.elapsed());

        let mut buf = String::with_capacity(128);
        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
        write_timestamp(&mut buf, now);
        buf.push(' ');

        let meta = span.metadata();
        let _ = write!(buf, "SPAN- {:<5} ", meta.level().as_str());
        buf.push_str(meta.target());
        buf.push('{');
        buf.push_str(meta.name());
        buf.push('}');

        // Duration
        if let Some(d) = dur {
            let _ = write!(buf, " dur={:.3}ms", d.as_secs_f64() * 1000.0);
        } else {
            buf.push_str(" dur=?");
        }

        buf.push('\n');

        for writer in &self.writers {
            let mut w = writer.lock().unwrap();
            let _ = w.write_all(buf.as_bytes());
        }
    }
}

fn write_timestamp(buf: &mut String, now: OffsetDateTime) {
    let _ = write!(
        buf,
        "[{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}{offset}]",
        year = now.year(),
        month = now.month() as u8,
        day = now.day(),
        hour = now.hour(),
        minute = now.minute(),
        second = now.second(),
        millis = now.millisecond(),
        offset = format_offset(now.offset()),
    );
}

fn format_offset(offset: UtcOffset) -> String {
    let seconds = offset.whole_seconds();
    let sign = if seconds < 0 { '-' } else { '+' };
    let seconds = seconds.abs();
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;

    format!("{sign}{hours:02}:{minutes:02}")
}

// ---------------------------------------------------------------------------
// SpanFieldVisitor — collects initial field values for `on_new_span`
// ---------------------------------------------------------------------------

/// Records the span's initial field values into a buffer as
/// ` key=value` pairs.
struct SpanFieldVisitor<'a> {
    buf: &'a mut String,
    first: bool,
}

impl<'a> tracing::field::Visit for SpanFieldVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if self.first { self.buf.push_str(": "); self.first = false; }
        else { self.buf.push(' '); }
        let _ = write!(self.buf, "{}={}", field.name(), field_format(value));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if self.first { self.buf.push_str(": "); self.first = false; }
        else { self.buf.push(' '); }
        let _ = write!(self.buf, "{}={}", field.name(), value);
    }
    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        if self.first { self.buf.push_str(": "); self.first = false; }
        else { self.buf.push(' '); }
        let _ = write!(self.buf, "{}={}", field.name(), value);
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        if self.first { self.buf.push_str(": "); self.first = false; }
        else { self.buf.push(' '); }
        let _ = write!(self.buf, "{}={}", field.name(), value);
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        if self.first { self.buf.push_str(": "); self.first = false; }
        else { self.buf.push(' '); }
        let _ = write!(self.buf, "{}={}", field.name(), value);
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        if self.first { self.buf.push_str(": "); self.first = false; }
        else { self.buf.push(' '); }
        let _ = write!(self.buf, "{}={}", field.name(), value);
    }
}

// ---------------------------------------------------------------------------
// EventVisitor — collects message + fields from a tracing Event
// ---------------------------------------------------------------------------

struct EventVisitor<'a> {
    buf: &'a mut String,
    first_field: bool,
}

impl<'a> tracing::field::Visit for EventVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.record_impl(field, |b| write!(b, "{}", field_format(value)))
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.record_impl(field, |b| { b.push_str(value); Ok(()) })
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.record_impl(field, |b| write!(b, "{}", value))
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.record_impl(field, |b| write!(b, "{}", value))
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.record_impl(field, |b| write!(b, "{}", value))
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.record_impl(field, |b| write!(b, "{}", value))
    }

    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        self.record_impl(field, |b| write!(b, "{}", value))
    }
}

impl EventVisitor<'_> {
    fn record_impl(
        &mut self,
        field: &tracing::field::Field,
        write_value: impl FnOnce(&mut String) -> std::fmt::Result,
    ) {
        let _ = if field.name() == "message" {
            self.first_field = false;
            write_value(self.buf)
        } else if self.first_field {
            self.buf.push_str(field.name());
            self.buf.push('=');
            self.first_field = false;
            write_value(self.buf)
        } else {
            self.buf.push(' ');
            self.buf.push_str(field.name());
            self.buf.push('=');
            write_value(self.buf)
        };
    }
}

// ---------------------------------------------------------------------------
// Target helpers
// ---------------------------------------------------------------------------

/// Flatten a `LogTarget::Composite` into a flat list of leaf targets.
/// Nested composites are expanded recursively.
fn flatten_targets(target: LogTarget) -> Vec<LogTarget> {
    match target {
        LogTarget::Composite(inner) => {
            let mut out = Vec::new();
            for t in inner {
                out.extend(flatten_targets(t));
            }
            out
        }
        leaf => vec![leaf],
    }
}

/// Open a `Box<dyn Write + Send>` for each leaf target.
fn open_writers(targets: Vec<LogTarget>) -> Result<Vec<Mutex<Box<dyn Write + Send>>>, LogError> {
    targets
        .into_iter()
        .map(|t| {
            let w: Box<dyn Write + Send> = match t {
                LogTarget::Stderr => Box::new(std::io::stderr()),
                LogTarget::File(path) => {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            LogError::create_dir(parent.display().to_string(), e)
                        })?;
                    }
                    let file = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                        .map_err(|e| LogError::open_file(path.display().to_string(), e))?;
                    Box::new(file)
                }
                LogTarget::Composite(_) => unreachable!("flatten_targets expands composites"),
            };
            Ok(Mutex::new(w))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format a value via `Debug`, stripping wrapping quotes from `str`/`String`.
fn field_format(value: &dyn std::fmt::Debug) -> String {
    let s = format!("{:?}", value);
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_owned()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};
    use tracing::{info, warn};
    use tracing_subscriber::prelude::*;

    /// A writer that captures output into a `Vec<u8>` for testing.
    struct TestWriter {
        buf: Arc<StdMutex<Vec<u8>>>,
    }

    impl TestWriter {
        fn new() -> Self {
            TestWriter {
                buf: Arc::new(StdMutex::new(Vec::new())),
            }
        }

        fn clone_buf(&self) -> Arc<StdMutex<Vec<u8>>> {
            self.buf.clone()
        }
    }

    impl std::io::Write for TestWriter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.buf.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn captured_layer_and_writer(
    ) -> (Arc<StdMutex<Vec<u8>>>, MkxpLayer) {
        let tw = TestWriter::new();
        let buf = tw.clone_buf();
        let layer = MkxpLayer {
            writers: vec![Mutex::new(Box::new(tw))],
            log_spans: false,
        };
        (buf, layer)
    }

    #[test]
    fn simple_info_event() {
        let (buf, layer) = captured_layer_and_writer();
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        info!("hello world");

        let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(out.contains("INFO"), "expected INFO in output, got: {}", out);
        assert!(out.contains("hello world"), "expected message, got: {}", out);
        assert!(out.ends_with('\n'), "expected trailing newline");
    }

    #[test]
    fn structured_fields_appear() {
        let (buf, layer) = captured_layer_and_writer();
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        info!(path = "Audio/BGM/battle.ogg", track = 0, "BGM start");

        let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(out.contains("BGM start"));
        assert!(out.contains("path=Audio/BGM/battle.ogg"));
        assert!(out.contains("track=0"));
    }

    #[test]
    fn level_padding() {
        let (buf, layer) = captured_layer_and_writer();
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        warn!("warning test");
        info!("info test");

        let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(out.contains("WARN "), "expected WARN  with padding, got: {}", out);
        assert!(out.contains("INFO "), "expected INFO  with padding, got: {}", out);
    }

    #[test]
    fn no_message_field_uses_first_field_as_message() {
        let (buf, layer) = captured_layer_and_writer();
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        info!(abc = "xyz");

        let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(out.contains("abc=xyz"), "expected field as message, got: {}", out);
    }

    #[test]
    fn file_target_creates_directory() {
        let dir = std::env::temp_dir().join("mkxp_log_test_layer2");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("test.log");

        let layer =
            MkxpLayer::new(LogTarget::File(path.clone()), false)
                .expect("create layer with file target");

        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);
        info!("file target test");

        assert!(path.exists(), "log file should exist");
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("file target test"), "expected log content in file");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn composite_writes_to_multiple_targets() {
        let dir = std::env::temp_dir().join("mkxp_log_test_composite");
        let _ = std::fs::remove_dir_all(&dir);
        let path_a = dir.join("a.log");
        let path_b = dir.join("b.log");

        let layer = MkxpLayer::new(LogTarget::Composite(vec![
            LogTarget::File(path_a.clone()),
            LogTarget::File(path_b.clone()),
        ]), false)
        .expect("create composite layer");

        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);
        info!("composite test message");

        for p in [&path_a, &path_b] {
            assert!(p.exists(), "{} should exist", p.display());
            let contents = std::fs::read_to_string(p).unwrap();
            assert!(
                contents.contains("composite test message"),
                "{} missing message, got: {}",
                p.display(),
                contents
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flatten_targets_expands_nested_composite() {
        let nested = LogTarget::Composite(vec![
            LogTarget::Stderr,
            LogTarget::Composite(vec![
                LogTarget::File("a.log".into()),
                LogTarget::File("b.log".into()),
            ]),
        ]);
        let flat = flatten_targets(nested);
        assert_eq!(flat.len(), 3);
        assert!(matches!(flat[0], LogTarget::Stderr));
        assert!(matches!(flat[1], LogTarget::File(_)));
        assert!(matches!(flat[2], LogTarget::File(_)));
    }

    #[test]
    fn flatten_single_target_is_unchanged() {
        let flat = flatten_targets(LogTarget::Stderr);
        assert_eq!(flat.len(), 1);
        assert!(matches!(flat[0], LogTarget::Stderr));

        let flat = flatten_targets(LogTarget::File("x.log".into()));
        assert_eq!(flat.len(), 1);
        assert!(matches!(flat[0], LogTarget::File(_)));
    }


    #[test]
    fn span_logs_creation_and_close_with_duration() {
        let (buf, layer) = captured_layer_and_writer_with_spans();
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        let span = tracing::info_span!("my_job", work = "rendering");
        let _enter = span.enter();
        info!("work done");

        // Explicitly drop to trigger on_close
        drop(_enter);
        drop(span);

        let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(out.contains("SPAN+"), "expected SPAN+ line, got: {}", out);
        assert!(out.contains("SPAN-"), "expected SPAN- line, got: {}", out);
        assert!(out.contains("my_job"), "expected span name, got: {}", out);
        assert!(out.contains("work=rendering"), "expected span fields, got: {}", out);
        assert!(out.contains("dur="), "expected duration, got: {}", out);
    }

    #[test]
    fn spans_disabled_by_default() {
        let (buf, layer) = captured_layer_and_writer(); // log_spans: false
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        let span = tracing::info_span!("silent_job");
        let _enter = span.enter();
        info!("only event");

        drop(_enter);
        drop(span);

        let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(!out.contains("SPAN+"), "SPAN+ should not appear");
        assert!(!out.contains("SPAN-"), "SPAN- should not appear");
        assert!(out.contains("only event"), "event should still appear");
    }

    fn captured_layer_and_writer_with_spans(
    ) -> (Arc<StdMutex<Vec<u8>>>, MkxpLayer) {
        let tw = TestWriter::new();
        let buf = tw.clone_buf();
        let layer = MkxpLayer {
            writers: vec![Mutex::new(Box::new(tw))],
            log_spans: true,
        };
        (buf, layer)
    }
}
