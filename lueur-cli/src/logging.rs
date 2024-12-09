use std::path::PathBuf;
use std::str::FromStr;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::never;
use tracing_log::LogTracer;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::SubscriberBuilder;

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
    log_file: &str,
) -> Result<WorkerGuard, Box<dyn std::error::Error>> {
    let path = log_file;
    let pathbuf = PathBuf::from_str(path).unwrap();
    // Create a rolling file appender that never rotates (i.e., always writes to
    // the same file)
    let file_appender =
        never(pathbuf.parent().unwrap(), pathbuf.file_name().unwrap());

    // Create a non-blocking, asynchronous writer to ensure logging does not
    // block your application
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // Build the subscriber with desired configurations
    let subscriber = SubscriberBuilder::default()
        .with_env_filter(EnvFilter::new("debug")) // Set log level to debug
        .with_writer(non_blocking) // Direct logs to the file
        .finish();

    // Set the subscriber as the global default
    tracing::subscriber::set_global_default(subscriber)?;

    // required so the messages from the ebpf programs get logged properly
    LogTracer::init()?;

    Ok(guard)
}
