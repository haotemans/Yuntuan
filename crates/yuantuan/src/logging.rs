use std::{collections::BTreeMap, path::Path};
use tracing::{field::{Field, Visit}, span::{Attributes, Id, Record}, Event, Subscriber};
use tracing_subscriber::{layer::{Context, SubscriberExt}, registry::LookupSpan, util::SubscriberInitExt, Layer};
use yuantuan_core::runtime_logs::RuntimeLogs;

#[derive(Default, Clone)]
struct Fields(BTreeMap<String, String>);
impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().into(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
    }
}

struct Capture(RuntimeLogs);
impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Capture {
    fn on_new_span(&self, attributes: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        attributes.record(&mut fields);
        if let Some(span) = ctx.span(id) { span.extensions_mut().insert(fields); }
    }
    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            if let Some(fields) = span.extensions_mut().get_mut::<Fields>() { values.record(fields); }
        }
    }
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(inherited) = span.extensions().get::<Fields>() { fields.0.extend(inherited.0.clone()); }
            }
        }
        event.record(&mut fields);
        let message = fields.0.remove("message").unwrap_or_default();
        let meta = event.metadata();
        self.0.record(meta.level().as_str(), meta.target(), &message, meta.file(), meta.line(), fields.0);
    }
}

pub struct Guard(RuntimeLogs);
impl Drop for Guard { fn drop(&mut self) { self.0.flush(); } }

pub fn init(logs: RuntimeLogs) -> Guard {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_target(false).with_filter(tracing_subscriber::filter::LevelFilter::INFO))
        .with(Capture(logs.clone()))
        .init();
    let previous = std::panic::take_hook();
    let panic_logs = logs.clone();
    std::panic::set_hook(Box::new(move |info| {
        let mut fields = BTreeMap::new();
        fields.insert("backtrace".into(), std::backtrace::Backtrace::force_capture().to_string());
        panic_logs.record("ERROR", "panic", &info.to_string(), info.location().map(|l|l.file()), info.location().map(|l|l.line()), fields);
        previous(info);
    }));
    Guard(logs)
}

pub fn register_secrets(logs: &RuntimeLogs, path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let value = if path.extension().is_some_and(|e|e=="json") {
        serde_json::from_str(&text).ok()
    } else {
        toml::from_str::<toml::Value>(&text).ok().and_then(|v|serde_json::to_value(v).ok())
    };
    if let Some(value) = value { logs.register_secrets(&value); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captures_error_chain_and_span_source() {
        let logs=RuntimeLogs::default();
        let subscriber=tracing_subscriber::registry().with(Capture(logs.clone()));
        tracing::subscriber::with_default(subscriber,|| {
            let span=tracing::info_span!("request",chat_id="42",source_msg_id=9);
            let _entered=span.enter();
            tracing::error!(error="provider returned HTTP 429","模型调用失败");
        });
        let entries=logs.entries();
        assert_eq!(entries.len(),1);
        assert_eq!(entries[0].fields["chat_id"],"42");
        assert_eq!(entries[0].fields["source_msg_id"],"9");
        assert!(entries[0].fields["error"].contains("429"));
    }
}
