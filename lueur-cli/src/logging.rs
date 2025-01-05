use std::path::PathBuf;
use std::str::FromStr;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::never;
use tracing_log::LogTracer;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::Registry;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;

/// Initializes the logging system to write exclusively to a debug text file
/// without rotation.
///
/// # Arguments
///
/// * `log_directory` - The directory where the log file will be stored.
/// * `log_file` - The name of the log file.
///
/// # Returns
///
/// * `Result<WorkerGuard, Box<dyn std::error::Error>>` - Returns a
///   `WorkerGuard` to ensure logs are flushed.
///
/// # Errors
///
/// Returns an error if the log directory cannot be created or if setting the
/// subscriber fails.
pub fn init_logging(
    log_file: Option<String>,
    enable_stdout: bool,
    log_level: Option<String>,
) -> Result<
    (Option<WorkerGuard>, Option<WorkerGuard>),
    Box<dyn std::error::Error>,
> {
    let mut fileguard: Option<WorkerGuard> = None;
    let mut stdoutguard: Option<WorkerGuard> = None;

    let mut layers = Vec::new();

    let log_level = match log_level {
        Some(log_level) => log_level,
        None => "debug,tower_http=debug".to_string(),
    };

    if let Some(log_file) = log_file {
        let path = log_file.as_str();
        let pathbuf = PathBuf::from_str(path).unwrap();
        let file_appender =
            never(pathbuf.parent().unwrap(), pathbuf.file_name().unwrap());

        let (file_non_blocking, file_guard) =
            tracing_appender::non_blocking(file_appender);

        fileguard = Some(file_guard);

        let file_filter = match EnvFilter::try_from_default_env() {
            Ok(filter) => filter,
            Err(_) => EnvFilter::builder().parse_lossy(log_level.clone()),
        };

        // Build the subscriber with desired configurations
        let file_layer = tracing_subscriber::fmt::layer()
            .with_span_events(FmtSpan::ACTIVE)
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

        let stdout_filter = match EnvFilter::try_from_default_env() {
            Ok(filter) => filter,
            Err(_) => EnvFilter::builder().parse_lossy(log_level.clone()),
        };

        stdoutguard = Some(stdout_guard);

        let stdout_layer: Box<dyn Layer<Registry> + Send + Sync> =
            tracing_subscriber::fmt::layer()
                .with_span_events(FmtSpan::NONE)
                .with_file(false)
                .with_line_number(true)
                .with_thread_ids(false)
                .with_target(true)
                .with_writer(stdout_non_blocking)
                .with_filter(stdout_filter)
                .boxed();

        layers.push(stdout_layer);
    }

    if !layers.is_empty() {
        let subscriber = tracing_subscriber::registry().with(layers);

        tracing::subscriber::set_global_default(subscriber)?;

        // required so the messages from the ebpf programs get logged properly
        LogTracer::init()?;
    }

    //Ok((file_guard, stdout_guard))
    Ok((fileguard, stdoutguard))
}
