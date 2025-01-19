use std::{path::PathBuf, str::FromStr};

use opentelemetry::{global, trace::TracerProvider as _, KeyValue};
use opentelemetry_sdk::{
    metrics::{MeterProviderBuilder, PeriodicReader, SdkMeterProvider},
    runtime,
    trace::{RandomIdGenerator, Sampler, Tracer, TracerProvider},
    Resource,
};
use opentelemetry_semantic_conventions::{
    attribute::{DEPLOYMENT_ENVIRONMENT_NAME, SERVICE_NAME, SERVICE_VERSION},
    SCHEMA_URL,
};
use tracing::{Level, Subscriber};
use tracing_appender::{non_blocking::WorkerGuard, rolling::never};
use tracing_log::LogTracer;
use tracing_opentelemetry::{MetricsLayer, OpenTelemetryLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer, Registry};
use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};

use crate::plugin::metrics;

// Create a Resource that captures information about the entity for which telemetry is recorded.
fn resource() -> Resource {
    Resource::from_schema_url(
        [
            KeyValue::new(SERVICE_NAME, env!("CARGO_PKG_NAME")),
            KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
            KeyValue::new(DEPLOYMENT_ENVIRONMENT_NAME, "dev"),
        ],
        SCHEMA_URL,
    )
}

// Construct MeterProvider for MetricsLayer
pub fn init_meter_provider() -> SdkMeterProvider {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_temporality(opentelemetry_sdk::metrics::Temporality::default())
        .build()
        .unwrap();

    let reader = PeriodicReader::builder(exporter, runtime::Tokio)
        .with_interval(std::time::Duration::from_secs(30))
        .build();

    // For debugging in development
    let stdout_reader = PeriodicReader::builder(
        opentelemetry_stdout::MetricExporter::default(),
        runtime::Tokio,
    )
    .build();

    let meter_provider = MeterProviderBuilder::default()
        .with_resource(resource())
        .with_reader(reader)
        .with_reader(stdout_reader)
        .build();

    global::set_meter_provider(meter_provider.clone());

    meter_provider
}

// Construct TracerProvider for OpenTelemetryLayer
pub fn init_tracer_provider() -> TracerProvider {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .build()
        .unwrap();

    TracerProvider::builder()
        // Customize sampling strategy
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
            1.0,
        ))))
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(resource())
        .with_batch_exporter(exporter, runtime::Tokio)
        .build()
}

pub fn shutdown_tracer(tracer_provider: TracerProvider, meter_provider: SdkMeterProvider) {
    tracer_provider.force_flush();
    let _ = meter_provider.force_flush();

    if let Err(err) = tracer_provider.shutdown() {
        eprintln!("{err:?}");
    }
    if let Err(err) = meter_provider.shutdown() {
        eprintln!("{err:?}");
    }
}


/// Combines logging and tracing/metrics layers into a single subscriber.
///
/// # Arguments
/// - `log_layers`: Layers for logging.
/// - `otel_layer`: Layer for OpenTelemetry tracing.
///
/// # Returns
/// A combined `tracing_subscriber` ready to be set as the global default.
pub fn init_subscriber(
    log_layers: Vec<Box<dyn tracing_subscriber::Layer<Registry> + Send + Sync>>,
    tracer_provider: &TracerProvider,
    meter_provider: &SdkMeterProvider
) -> Result<(), Box<dyn std::error::Error>> {
    let registry = tracing_subscriber::registry();
    
    let mut layers = Vec::new();
    layers.extend(log_layers);

    let tracer = tracer_provider.tracer("lueur");
    let telemetry = OpenTelemetryLayer::new(tracer)
        .with_error_records_to_exceptions(true);
    let metrics = MetricsLayer::new(meter_provider.clone());

    let subscriber = registry.with(layers).with(telemetry).with(metrics);

    tracing::subscriber::set_global_default(subscriber)?;

    // required so the messages from the ebpf programs get logged properly
    LogTracer::init()?;

    Ok(())
}


/// Sets up file and stdout logging layers.
///
/// # Arguments
/// - `log_file`: Optional path to the log file.
/// - `enable_stdout`: Whether to log to stdout.
/// - `log_level`: The desired log level filter (e.g., "debug").
///
/// # Returns
/// A tuple containing optional guards for file and stdout logging layers.
pub fn setup_logging(
    log_file: Option<String>,
    enable_stdout: bool,
    log_level: Option<String>,
) -> Result<
    (Option<WorkerGuard>, Option<WorkerGuard>, Vec<Box<dyn tracing_subscriber::Layer<Registry> + Send + Sync>>),
    Box<dyn std::error::Error>,
> {
    let mut fileguard: Option<WorkerGuard> = None;
    let mut stdoutguard: Option<WorkerGuard> = None;
    let mut layers = Vec::new();

    let log_level = log_level.unwrap_or_else(|| "debug,tower_http=debug,otel::tracing=info".to_string());

    if let Some(log_file) = log_file {
        let path = log_file.as_str();
        let pathbuf = PathBuf::from_str(path).unwrap();
        let file_appender =
            never(pathbuf.parent().unwrap(), pathbuf.file_name().unwrap());

        let (file_non_blocking, file_guard) =
            tracing_appender::non_blocking(file_appender);

        fileguard = Some(file_guard);

        let file_filter = EnvFilter::builder().parse_lossy(log_level.clone());

        let file_layer = tracing_subscriber::fmt::layer()
            .with_file(false)
            .with_line_number(true)
            .with_thread_ids(false)
            .with_target(true)
            .with_writer(file_non_blocking)
            .with_filter(file_filter)
            .boxed();

        layers.push(file_layer);
    }

    if enable_stdout {
        let (stdout_non_blocking, stdout_guard) =
            tracing_appender::non_blocking(std::io::stdout());

        let stdout_filter = EnvFilter::builder().parse_lossy(log_level.clone());

        stdoutguard = Some(stdout_guard);

        let stdout_layer = tracing_subscriber::fmt::layer()
            .with_file(false)
            .with_line_number(true)
            .with_thread_ids(false)
            .with_target(true)
            .with_writer(stdout_non_blocking)
            .with_filter(stdout_filter)
            .boxed();

        layers.push(stdout_layer);
    }

    Ok((fileguard, stdoutguard, layers))
}
