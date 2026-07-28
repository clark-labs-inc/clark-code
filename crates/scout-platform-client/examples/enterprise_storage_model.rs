//! Deterministic Scout authority/projection storage and request-cost model.
//!
//! Prices are explicit inputs, not hidden constants. Defaults are documented
//! US East (N. Virginia) public list-price assumptions captured 2026-07-26.

use serde::Serialize;

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Clone, Copy)]
struct Prices {
    s3_standard_gb_month: f64,
    s3_put_per_1k: f64,
    aurora_standard_gb_month: f64,
    aurora_io_per_million: f64,
    dynamodb_gb_month: f64,
    dynamodb_write_per_million: f64,
    opensearch_managed_gb_month: f64,
}

impl Default for Prices {
    fn default() -> Self {
        Self {
            s3_standard_gb_month: 0.023,
            s3_put_per_1k: 0.005,
            aurora_standard_gb_month: 0.10,
            aurora_io_per_million: 0.20,
            dynamodb_gb_month: 0.25,
            dynamodb_write_per_million: 0.625,
            opensearch_managed_gb_month: 0.02,
        }
    }
}

#[derive(Serialize)]
struct Assumptions {
    captured_at: &'static str,
    region: &'static str,
    collectors: u64,
    microservices: u64,
    observations_per_evidence_object: u64,
    compressed_evidence_bytes_per_observation: u64,
    aurora_bytes_per_observation: u64,
    aurora_io_per_observation: u64,
    dynamodb_projection_bytes_per_observation: u64,
    dynamodb_writes_per_observation: u64,
    opensearch_projection_bytes_per_observation: u64,
    target_ingest_hours: u64,
    prices: PriceReceipt,
}

#[derive(Serialize)]
struct PriceReceipt {
    s3_standard_gb_month_usd: f64,
    s3_put_per_1k_usd: f64,
    aurora_standard_gb_month_usd: f64,
    aurora_io_per_million_usd: f64,
    dynamodb_gb_month_usd: f64,
    dynamodb_write_per_million_usd: f64,
    opensearch_managed_gb_month_usd: f64,
    sources: Vec<&'static str>,
}

#[derive(Serialize)]
struct Scale {
    observations: u64,
    evidence_objects: u64,
    observations_per_second_for_target: f64,
    observations_per_collector: f64,
    s3_evidence_gib: f64,
    s3_billable_gb: f64,
    aurora_authority_gib: f64,
    aurora_billable_gb: f64,
    dynamodb_projection_gib: f64,
    dynamodb_billable_gb: f64,
    opensearch_projection_gib: f64,
    opensearch_billable_gb: f64,
    estimated_first_month_usd: Cost,
    estimated_steady_storage_month_usd: Cost,
}

#[derive(Serialize)]
struct Cost {
    s3: f64,
    aurora_storage_and_io: f64,
    dynamodb_projection: f64,
    opensearch_storage_only: f64,
    required_authority_total: f64,
    with_dynamodb_projection: f64,
    with_dynamodb_and_opensearch_storage: f64,
}

#[derive(Serialize)]
struct Receipt {
    schema: &'static str,
    status: &'static str,
    authority: &'static str,
    evidence_store: &'static str,
    cost_scope: &'static str,
    projection_policy: ProjectionPolicy,
    assumptions: Assumptions,
    scales: Vec<Scale>,
}

#[derive(Serialize)]
struct ProjectionPolicy {
    dynamodb: &'static str,
    opensearch: &'static str,
    neptune: &'static str,
    note: &'static str,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Scout enterprise storage model failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let prices = Prices::default();
    let assumptions = Assumptions {
        captured_at: "2026-07-26",
        region: "us-east-1",
        collectors: 10_000,
        microservices: 1_000,
        observations_per_evidence_object: 100,
        compressed_evidence_bytes_per_observation: 960,
        aurora_bytes_per_observation: 1_024,
        aurora_io_per_observation: 6,
        dynamodb_projection_bytes_per_observation: 512,
        dynamodb_writes_per_observation: 2,
        opensearch_projection_bytes_per_observation: 384,
        target_ingest_hours: 6,
        prices: PriceReceipt {
            s3_standard_gb_month_usd: prices.s3_standard_gb_month,
            s3_put_per_1k_usd: prices.s3_put_per_1k,
            aurora_standard_gb_month_usd: prices.aurora_standard_gb_month,
            aurora_io_per_million_usd: prices.aurora_io_per_million,
            dynamodb_gb_month_usd: prices.dynamodb_gb_month,
            dynamodb_write_per_million_usd: prices.dynamodb_write_per_million,
            opensearch_managed_gb_month_usd: prices.opensearch_managed_gb_month,
            sources: vec![
                "https://aws.amazon.com/s3/pricing/",
                "https://aws.amazon.com/rds/aurora/pricing/",
                "https://aws.amazon.com/dynamodb/pricing/",
                "https://aws.amazon.com/opensearch-service/pricing/",
            ],
        },
    };
    let scales = [10_000_000, 100_000_000, 1_000_000_000]
        .into_iter()
        .map(|observations| scale(observations, &assumptions, prices))
        .collect();
    let receipt = Receipt {
        schema: "clark-system-cartography-enterprise-storage-model-v1",
        status: "modeled",
        authority: "aurora-postgresql",
        evidence_store: "s3-versioned-sse-kms",
        cost_scope: "Storage, S3 PUT, Aurora Standard I/O, and DynamoDB on-demand writes only. Excludes Aurora compute, KMS, backups/PITR, S3 reads/retrieval/transfer, DynamoDB reads, and all OpenSearch/Neptune compute.",
        projection_policy: ProjectionPolicy {
            dynamodb: "benchmark-gated optional current-state/read projection",
            opensearch: "benchmark-gated optional text/faceted retrieval projection",
            neptune: "disabled until measured graph-query gain exceeds PostgreSQL by 10x within an explicit monthly budget",
            note: "Projection costs exclude compute where request shape, duty cycle, and measured OCU/NCU demand are not yet available.",
        },
        assumptions,
        scales,
    };
    let body = serde_json::to_string_pretty(&receipt).map_err(|error| error.to_string())?;
    println!("{body}");
    Ok(())
}

fn scale(observations: u64, assumptions: &Assumptions, prices: Prices) -> Scale {
    let evidence_objects = observations.div_ceil(assumptions.observations_per_evidence_object);
    let s3_bytes =
        observations as f64 * assumptions.compressed_evidence_bytes_per_observation as f64;
    let aurora_bytes = observations as f64 * assumptions.aurora_bytes_per_observation as f64;
    let dynamo_bytes =
        observations as f64 * assumptions.dynamodb_projection_bytes_per_observation as f64;
    let opensearch_bytes =
        observations as f64 * assumptions.opensearch_projection_bytes_per_observation as f64;
    let s3_gib = s3_bytes / GIB;
    let aurora_gib = aurora_bytes / GIB;
    let dynamo_gib = dynamo_bytes / GIB;
    let opensearch_gib = opensearch_bytes / GIB;
    let s3_billable_gb = s3_bytes / 1_000_000_000.0;
    let aurora_billable_gb = aurora_bytes / 1_000_000_000.0;
    let dynamo_billable_gb = dynamo_bytes / 1_000_000_000.0;
    let opensearch_billable_gb = opensearch_bytes / 1_000_000_000.0;
    let s3_storage = s3_billable_gb * prices.s3_standard_gb_month;
    let s3_put = evidence_objects as f64 / 1_000.0 * prices.s3_put_per_1k;
    let aurora_storage = aurora_billable_gb * prices.aurora_standard_gb_month;
    let aurora_io = observations as f64 * assumptions.aurora_io_per_observation as f64
        / 1_000_000.0
        * prices.aurora_io_per_million;
    let dynamo_storage = dynamo_billable_gb * prices.dynamodb_gb_month;
    let dynamo_writes = observations as f64 * assumptions.dynamodb_writes_per_observation as f64
        / 1_000_000.0
        * prices.dynamodb_write_per_million;
    let opensearch_storage = opensearch_billable_gb * prices.opensearch_managed_gb_month;
    let first = costs(
        s3_storage + s3_put,
        aurora_storage + aurora_io,
        dynamo_storage + dynamo_writes,
        opensearch_storage,
    );
    let steady = costs(
        s3_storage,
        aurora_storage,
        dynamo_storage,
        opensearch_storage,
    );
    Scale {
        observations,
        evidence_objects,
        observations_per_second_for_target: observations as f64
            / (assumptions.target_ingest_hours * 3_600) as f64,
        observations_per_collector: observations as f64 / assumptions.collectors as f64,
        s3_evidence_gib: s3_gib,
        s3_billable_gb,
        aurora_authority_gib: aurora_gib,
        aurora_billable_gb,
        dynamodb_projection_gib: dynamo_gib,
        dynamodb_billable_gb: dynamo_billable_gb,
        opensearch_projection_gib: opensearch_gib,
        opensearch_billable_gb,
        estimated_first_month_usd: first,
        estimated_steady_storage_month_usd: steady,
    }
}

fn costs(s3: f64, aurora: f64, dynamodb: f64, opensearch: f64) -> Cost {
    Cost {
        s3,
        aurora_storage_and_io: aurora,
        dynamodb_projection: dynamodb,
        opensearch_storage_only: opensearch,
        required_authority_total: s3 + aurora,
        with_dynamodb_projection: s3 + aurora + dynamodb,
        with_dynamodb_and_opensearch_storage: s3 + aurora + dynamodb + opensearch,
    }
}

#[cfg(test)]
mod tests {
    use super::{scale, Assumptions, PriceReceipt, Prices};

    #[test]
    fn costs_and_throughput_scale_linearly() {
        let prices = Prices::default();
        let assumptions = Assumptions {
            captured_at: "test",
            region: "test",
            collectors: 10_000,
            microservices: 1_000,
            observations_per_evidence_object: 100,
            compressed_evidence_bytes_per_observation: 960,
            aurora_bytes_per_observation: 1_024,
            aurora_io_per_observation: 6,
            dynamodb_projection_bytes_per_observation: 512,
            dynamodb_writes_per_observation: 2,
            opensearch_projection_bytes_per_observation: 384,
            target_ingest_hours: 6,
            prices: PriceReceipt {
                s3_standard_gb_month_usd: 0.0,
                s3_put_per_1k_usd: 0.0,
                aurora_standard_gb_month_usd: 0.0,
                aurora_io_per_million_usd: 0.0,
                dynamodb_gb_month_usd: 0.0,
                dynamodb_write_per_million_usd: 0.0,
                opensearch_managed_gb_month_usd: 0.0,
                sources: Vec::new(),
            },
        };
        let small = scale(10_000_000, &assumptions, prices);
        let large = scale(100_000_000, &assumptions, prices);
        assert_eq!(small.evidence_objects * 10, large.evidence_objects);
        assert!(
            (small.observations_per_second_for_target * 10.0
                - large.observations_per_second_for_target)
                .abs()
                < f64::EPSILON
        );
    }
}
