from rust:1.72 as builder

workdir /tmp
COPY Cargo.* ./
COPY src/ ./src/
RUN cargo install --path .

entrypoint ["nomad-otel-metrics-scraper"]
