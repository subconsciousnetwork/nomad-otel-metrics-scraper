use anyhow::Result;
use clap::Parser;
use humantime::Duration;
use log::{error, info, };
use opentelemetry::{metrics::MeterProvider as _, KeyValue, StringValue, Value};
use opentelemetry_sdk::{
    metrics::{MeterProvider, PeriodicReader},
    runtime,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
use url::Url;

#[derive(Parser)]
#[command(version, about, name = "nomad-otel-metrics-scraper")]
pub struct Cli {
    #[clap(short, long, default_value = "http://localhost:4646")]
    pub nomad_url: Url,

    #[clap(short, long, default_value = "60s")]
    pub poll_interval: Duration,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let args = Cli::parse();
    info!("Polling {} every {}", args.nomad_url, args.poll_interval);

    let meter_provider = setup_otel();
    let closable_meter_provider = meter_provider.clone();
    let meter = meter_provider.meter("nomad_metrics");

    let status_ratio = meter
        .f64_observable_gauge("nomad_job_status_ratio")
        .with_description("The ratio of working relative to expected count for each nomad job")
        .init();

    let service_up = meter.u64_observable_gauge("nomad_job_up").init();
    let service_down = meter.u64_observable_gauge("nomad_job_down").init();

    let job_metric_map = Arc::new(Mutex::new(HashMap::<String, StatusCount>::new()));

    let cancel_token = CancellationToken::new();
    let status_checker_token = cancel_token.clone();

    let status_loop = tokio::spawn(async move {
        loop {

            tokio::select! {
                _ = status_checker_token.cancelled() => {
                    return
                }
                _ = tokio::time::sleep(args.poll_interval.into()) => {
                    info!("Doing some important async work here");
                }
            };
        }
    });

    tokio::spawn(async move {
        // wait for ctrlc
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                info!("Provider, as we know it. {:#?}", closable_meter_provider);
                info!("Flushing metrics.");
                // opentelemetry::global::shutdown_meter_provider()
                closable_meter_provider.force_flush()?;
                info!("Shutting it down.");
                closable_meter_provider.shutdown()?;
                info!("Meter provider is shutdown");
                cancel_token.cancel();
                Ok::<(), anyhow::Error>(())
            }
            Err(err) => {
                error!("Unable to listen for shutdown signal. Ending. {}", err);
                cancel_token.cancel();
                Ok(())
            }
        }
    });

    meter.register_callback(
        &[
            service_up.as_any(),
            service_down.as_any(),
            status_ratio.as_any(),
        ],
        move |observer| {
            let data = job_metric_map.lock().unwrap();
            for (job_name, status_count) in data.iter() {
                let labels = [KeyValue::new(
                    "job",
                    Value::String(StringValue::from(job_name.to_owned())),
                )];

                observer.observe_u64(&service_up, status_count.up, &labels);
                observer.observe_u64(&service_down, status_count.down, &labels);
                observer.observe_f64(&status_ratio, status_count.up_ratio, &labels);
            }
        },
    )?;

    status_loop.await?;

    Ok(())
}

fn setup_otel() -> Arc<MeterProvider> {
    let exporter = opentelemetry_stdout::MetricsExporterBuilder::default()
        // uncomment the below lines to pretty print output.
        .with_encoder(|writer, data| Ok(serde_json::to_writer_pretty(writer, &data).unwrap()))
        .build();
    // TODO: Setup service name
    let


        reader = PeriodicReader::builder(exporter, runtime::Tokio)
        .with_interval(std::time::Duration::from_millis(15_000)) // millis
        .build();
    Arc::new(
        MeterProvider::builder()
            .with_reader(reader)
            .build(),
    )
}

#[derive(Debug)]
struct StatusCount {
    up: u64,
    down: u64,
    up_ratio: f64,
}
