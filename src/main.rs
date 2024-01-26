use anyhow::Result;
use clap::Parser;
use humantime::Duration;
use log::{debug, error, info, trace};
use opentelemetry::{metrics::MeterProvider as _, KeyValue, StringValue, Value};
use opentelemetry_sdk::{
    Resource,
    metrics::{MeterProvider, PeriodicReader},
    runtime,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
use url::Url;
use opentelemetry_sdk::metrics::reader::{
    DefaultAggregationSelector, DefaultTemporalitySelector,
};

#[derive(Parser)]
#[command(version, about, name = "nomad-otel-metrics-scraper")]
pub struct Cli {
    /// URL of the nomad instance to contact
    #[clap(short = 'u', long, default_value = "http://localhost:4646")]
    pub nomad_url: Url,

    /// How often to query nomad
    #[clap(short, long, default_value = "60s")]
    pub nomad_poll_interval: Duration,

    /// Whether to print out the metrics we are publishing to stdout
    #[clap(long)]
    pub debug: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let args = Cli::parse();
    info!("Polling {} every {}", args.nomad_url, args.nomad_poll_interval);

    let meter_provider = setup_otel(args.debug)?;
    let closable_meter_provider = meter_provider.clone();
    let meter = meter_provider.meter("nomad_metrics");

    let status_ratio = meter
        .f64_observable_gauge("nomad_job_status_ratio")
        .with_description("The ratio of working relative to expected count for each nomad job")
        .init();

    let service_up = meter.u64_observable_gauge("nomad_job_up").init();
    let service_down = meter.u64_observable_gauge("nomad_job_down").init();

    let nomad_url = args.nomad_url.to_string();

    let job_metric_map = Arc::new(Mutex::new(HashMap::<String, StatusCount>::new()));
    let looper_job_metric_map = job_metric_map.clone();

    let cancel_token = CancellationToken::new();
    let status_checker_token = cancel_token.clone();

    let status_loop = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = status_checker_token.cancelled() => {
                    return
                }
                _ = tokio::time::sleep(args.nomad_poll_interval.into()) => {
                // TODO: Set timeouts
                let statuses = get_statuses_for_jobs(nomad_url.clone())
                    .await
                    .expect("Unable to fetch statuses from the provided domain");

                {
                    // in it's own scope so we don't keep the lock for too long.
                    let mut data = looper_job_metric_map.lock().unwrap();
                    for (job_name, status) in statuses.iter() {
                        let status_count = StatusCount {
                            up: status.healthy.into(),
                            down: status.unhealthy.into(),
                            up_ratio: (status.healthy / status.desired).into(),
                        };

                        debug!("Job {} had status {:?}", job_name, status_count);
                        data.insert(job_name.clone(), status_count);
                    }
                }

                }
            };
        }
    });

    tokio::spawn(async move {
        // wait for ctrlc
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                // TODO: This should really be blow the metric provider closing,
                // b/c it just hard-stops the binary. Unfortunately, the
                // shutdown mechanism below hangs. Pending
                // https://cloud-native.slack.com/archives/C03GDP0H023/p1706210769680649
                cancel_token.cancel();

                trace!("Provider, as we know it. {:#?}", closable_meter_provider);
                info!("Flushing metrics.");
                closable_meter_provider.force_flush()?;
                info!("Shutting it down.");
                closable_meter_provider.shutdown()?;
                info!("Meter provider is shutdown");
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
                    // NB: "job" is a reserved word for these.
                    "nomad_job",
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

fn setup_otel(debug: bool) -> Result<Arc<MeterProvider>> {

    let mut builder =         MeterProvider::builder()
        .with_resource(Resource::new(vec![KeyValue::new("service.name", "nomad-scraper")]));

    if debug {
        builder = builder.with_reader(PeriodicReader::builder(
            opentelemetry_stdout::MetricsExporterBuilder::default()
                .with_encoder(|writer, data| {
                    serde_json::to_writer_pretty(writer, &data).unwrap();
                    Ok(())
                })
                .build(),
            runtime::Tokio)
                                      .build());
    }


    let builder = builder.with_reader(PeriodicReader::builder(
        opentelemetry_otlp::new_exporter()
            .http()
            .build_metrics_exporter(
                Box::new(DefaultAggregationSelector::new()),
                Box::new(DefaultTemporalitySelector::new()),
            )?,
        runtime::Tokio
    )
                                      .build());

    Ok(Arc::new(builder.build()))
}

async fn get_statuses_for_jobs(nomad_url: String) -> Result<Vec<(String, JobScaleStatus)>> {
    let client = Client::new();
    let entries = client
        .get(format!("{}v1/jobs", nomad_url))
        .send()
        .await?
        .json::<Vec<JobListEntry>>()
        .await?;
    let mut statuses = Vec::new();
    for entry in entries.iter() {
        let job_name = &entry.name;
        trace!("Looking up status for {}..", job_name);
        let job_scale = client
            .get(format!("{}v1/job/{}/scale", nomad_url, job_name))
            .send()
            .await?
            .json::<JobScale>()
            .await?;
        for (name, job_status) in job_scale.task_groups.iter() {
            // TODO: This should likely yield, but I'm not entirely sure how to accomplish that w/ Result.
            statuses.push((name.to_owned(), job_status.to_owned()));
        }
    }
    Ok(statuses)
}

#[derive(Serialize, Deserialize, Debug)]
struct JobListEntry {
    #[serde(rename = "Name")]
    name: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct JobScale {
    #[serde(rename = "TaskGroups")]
    task_groups: HashMap<String, JobScaleStatus>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct JobScaleStatus {
    #[serde(rename = "Desired")]
    desired: u32,
    #[serde(rename = "Healthy")]
    healthy: u32,
    #[serde(rename = "Placed")]
    placed: u32,
    #[serde(rename = "Running")]
    running: u32,
    #[serde(rename = "Unhealthy")]
    unhealthy: u32,
}

#[derive(Debug)]
struct StatusCount {
    up: u64,
    down: u64,
    up_ratio: f64,
}
