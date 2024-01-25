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

```
nomad-otel-metrics-scraper --nomad-url http://my.nomad.instance/ --poll-interval 15s --debug
```

The debug flag sets the stdout exporter for OTel so you can see what metics it's writing.

The OTel portions conform to the [Environment Variable Specification](https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/) and can be adjusted accordingly.
