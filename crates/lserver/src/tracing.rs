use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{filter::EnvFilter, fmt::format::FmtSpan, prelude::*};

pub fn setup_tracing() -> WorkerGuard {
    // 1. Create a rolling file appender
    //    Logs will be saved to 'logs/my_app.log' and rotated daily.
    //    'Rotation::DAILY' can be 'Rotation::HOURLY', 'Rotation::NEVER', or 'Rotation::MINUTELY'.
    let file_appender = RollingFileAppender::new(
        Rotation::DAILY,
        "/Users/maheshbansod/projects/myls/logs",
        "my_app.log",
    );

    // 2. You can also split output between stdout and a file if needed.
    //    This creates a non-blocking writer for the file appender.
    let (non_blocking_file_writer, guard) = tracing_appender::non_blocking(file_appender);

    // 3. Configure the tracing subscriber
    tracing_subscriber::registry()
        // Add an EnvFilter to control log levels via an environment variable (e.g., RUST_LOG)
        .with(EnvFilter::from_default_env())
        // Add a formatter for the file output. You can use `Json` or `Full` for more structured/detailed logs.
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(non_blocking_file_writer)
                .with_ansi(false) // Disable ANSI escape codes for file output
                .with_span_events(FmtSpan::CLOSE) // Log when spans close
                .compact(), // Optional: Output logs in JSON format for easier parsing
        )
        // Optional: Add a console layer for simultaneous console output
        // .with(
        //     tracing_subscriber::fmt::layer()
        //         .with_writer(io::stdout) // Output to stdout
        //         .with_ansi(true) // Enable ANSI colors for console output
        //         .with_span_events(FmtSpan::CLOSE),
        // )
        .init();
    return guard;
}
