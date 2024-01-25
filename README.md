# Gather job metrics from nomad

Nomad does not yet expose job-specific metrics (see [this
issue](https://github.com/hashicorp/nomad/issues/16602)). This means it's not
possible to set alarms like "Hey, if my jobs have more than 2 replicas down,
alert."

This tool fixes that issue by querying the nomad API for job-specific metrics
and writing those to an [OTLP](https://opentelemetry.io/docs/specs/otel/protocol/) agent.

I got the idea from https://github.com/strigo/nomad-service-discovery-exporter,
but it uses prometheus rather than OTLP.


## Usage

```text
Usage: nomad-otel-metrics-scraper [OPTIONS]

Options:
  -u, --nomad-url <NOMAD_URL>
          URL of the nomad instance to contact [default: http://localhost:4646]
  -n, --nomad-poll-interval <NOMAD_POLL_INTERVAL>
          How often to query nomad [default: 60s]
      --debug
          Whether to print out the metrics we are publishing to stdout
  -h, --help
          Print help
  -V, --version
          Print version
```

The OTel portions conform to the [Environment Variable Specification](https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/) and can be adjusted accordingly.
