use anyhow::Result;
use clap::Parser;

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "qhook=info".into());

    #[cfg(feature = "otel")]
    init_otel_tracing(filter);

    #[cfg(not(feature = "otel"))]
    init_std_tracing(filter);

    let args = qhook::cli::Args::parse();
    args.run().await
}

fn init_std_tracing(filter: tracing_subscriber::EnvFilter) {
    if std::env::var("QHOOK_LOG_FORMAT").as_deref() == Ok("json") {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
}

/// Initialize tracing with OpenTelemetry OTLP exporter.
/// Falls back to standard tracing if OTEL_EXPORTER_OTLP_ENDPOINT is not set.
#[cfg(feature = "otel")]
fn init_otel_tracing(filter: tracing_subscriber::EnvFilter) {
    use opentelemetry::global;
    use opentelemetry::trace::TracerProvider;
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_err() {
        init_std_tracing(filter);
        return;
    }

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .build()
        .expect("Failed to create OTLP exporter");

    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_resource(opentelemetry_sdk::Resource::new(vec![
            opentelemetry::KeyValue::new("service.name", "qhook"),
        ]))
        .build();

    global::set_tracer_provider(provider.clone());
    let tracer = provider.tracer("qhook");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    let fmt_layer = if std::env::var("QHOOK_LOG_FORMAT").as_deref() == Ok("json") {
        tracing_subscriber::fmt::layer()
            .json()
            .with_target(true)
            .boxed()
    } else {
        tracing_subscriber::fmt::layer().boxed()
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(otel_layer)
        .init();

    tracing::info!("OpenTelemetry tracing enabled");
}
