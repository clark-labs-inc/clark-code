use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::contract::{HarnessKind, TaskId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authorization {
    None,
    UserRequested,
    RepositoryPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationPurpose {
    Explore,
    Review,
    Verify,
    ExternalResearch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelRate {
    pub input_per_million_usd: f64,
    pub output_per_million_usd: f64,
}

impl ModelRate {
    pub fn projected_cost(self, input_tokens: u64, output_tokens: u64) -> f64 {
        input_tokens as f64 / 1_000_000.0 * self.input_per_million_usd
            + output_tokens as f64 / 1_000_000.0 * self.output_per_million_usd
    }

    fn valid(self) -> bool {
        self.input_per_million_usd.is_finite()
            && self.output_per_million_usd.is_finite()
            && self.input_per_million_usd >= 0.0
            && self.output_per_million_usd >= 0.0
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkstreamEstimate {
    pub task_id: TaskId,
    pub scopes: BTreeSet<String>,
    pub estimated_context_tokens: u64,
    pub estimated_output_tokens: u64,
    pub harness_kind: HarnessKind,
    pub model: String,
    pub model_rate: Option<ModelRate>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RiskSignals {
    pub changed_paths: usize,
    pub touches_public_api: bool,
    pub touches_auth_or_security: bool,
    pub touches_data_migration: bool,
    pub touches_dependencies: bool,
    pub touches_concurrency: bool,
    pub verification_missing: bool,
    pub user_requested_review: bool,
    pub prior_attempt_failed: bool,
}

impl RiskSignals {
    pub fn requires_independent_gate(&self) -> bool {
        self.user_requested_review
            || self.prior_attempt_failed
            || self.verification_missing
            || self.touches_auth_or_security
            || self.touches_data_migration
            || self.touches_concurrency
            || self.touches_public_api
            || self.touches_dependencies
            || self.changed_paths >= 8
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdmissionRequest {
    pub authorization: Authorization,
    pub purpose: OrchestrationPurpose,
    pub workstreams: Vec<WorkstreamEstimate>,
    pub root_model: String,
    pub root_model_rate: ModelRate,
    pub root_estimated_output_tokens: u64,
    pub risk: RiskSignals,
    pub external_research_required: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdmissionPolicy {
    pub max_agents: usize,
    pub minimum_parallel_context_tokens: u64,
    pub child_system_prompt_tokens: u64,
    pub max_projected_cost_ratio: f64,
    /// Optional absolute budget for the first parallel batch. Inputs count 1x;
    /// projected outputs use `output_token_weight` to reflect their higher cost.
    pub max_projected_weighted_tokens: Option<f64>,
    pub output_token_weight: f64,
    pub require_explicit_authorization: bool,
}

impl Default for AdmissionPolicy {
    fn default() -> Self {
        Self {
            max_agents: 3,
            minimum_parallel_context_tokens: 40_000,
            child_system_prompt_tokens: 6_000,
            max_projected_cost_ratio: 1.25,
            max_projected_weighted_tokens: None,
            output_token_weight: 4.0,
            require_explicit_authorization: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rejection {
    pub code: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdmissionDecision {
    pub admitted: bool,
    pub rejections: Vec<Rejection>,
    pub projected_single_cost_usd: f64,
    pub projected_multi_cost_usd: f64,
    pub projected_cost_ratio: f64,
    pub projected_weighted_tokens: f64,
}

impl AdmissionPolicy {
    pub fn evaluate(&self, request: &AdmissionRequest) -> AdmissionDecision {
        let mut rejections = Vec::new();
        if self.require_explicit_authorization && request.authorization == Authorization::None {
            reject(
                &mut rejections,
                "authorization_required",
                "delegation requires a user request or repository policy",
            );
        }
        if request.workstreams.is_empty() || request.workstreams.len() > self.max_agents {
            reject(
                &mut rejections,
                "invalid_fanout_size",
                "workstream count is outside the configured bounds",
            );
        }
        if !request.root_model_rate.valid()
            || request
                .workstreams
                .iter()
                .filter_map(|workstream| workstream.model_rate)
                .any(|rate| !rate.valid())
        {
            reject(
                &mut rejections,
                "invalid_model_rate",
                "model rates must be finite and non-negative",
            );
        }
        if !self.output_token_weight.is_finite() || self.output_token_weight < 1.0 {
            reject(
                &mut rejections,
                "invalid_token_weight",
                "output token weight must be finite and at least one",
            );
        }
        if self
            .max_projected_weighted_tokens
            .is_some_and(|limit| !limit.is_finite() || limit <= 0.0)
        {
            reject(
                &mut rejections,
                "invalid_token_budget",
                "projected token budget must be finite and greater than zero",
            );
        }

        let context_tokens = request
            .workstreams
            .iter()
            .map(|workstream| workstream.estimated_context_tokens)
            .sum::<u64>();
        match request.purpose {
            OrchestrationPurpose::Explore => {
                if request.workstreams.len() < 2 {
                    reject(
                        &mut rejections,
                        "not_parallel",
                        "exploration fan-out requires at least two independent workstreams",
                    );
                }
                if context_tokens < self.minimum_parallel_context_tokens {
                    reject(
                        &mut rejections,
                        "context_too_small",
                        "the repository slice is too small to justify repeated agent prompts",
                    );
                }
                if let Some(overlap) = overlapping_scope(&request.workstreams) {
                    reject(
                        &mut rejections,
                        "overlapping_scopes",
                        &format!("workstreams do not have independent scopes: {overlap}"),
                    );
                }
            }
            OrchestrationPurpose::Review | OrchestrationPurpose::Verify => {
                if request.workstreams.len() != 1 {
                    reject(
                        &mut rejections,
                        "gate_must_be_single",
                        "review and verification gates use one independent agent",
                    );
                }
                if !request.risk.requires_independent_gate() {
                    reject(
                        &mut rejections,
                        "risk_gate_not_triggered",
                        "the change does not meet the independent-review risk threshold",
                    );
                }
            }
            OrchestrationPurpose::ExternalResearch => {
                if !request.external_research_required {
                    reject(
                        &mut rejections,
                        "external_research_not_required",
                        "product cloud is reserved for genuinely external research",
                    );
                }
                if request.workstreams.len() != 1
                    || request.workstreams[0].harness_kind != HarnessKind::BrokeredCloud
                {
                    reject(
                        &mut rejections,
                        "invalid_cloud_route",
                        "external research must use exactly one product cloud workstream",
                    );
                }
            }
        }
        if request.purpose != OrchestrationPurpose::ExternalResearch
            && request
                .workstreams
                .iter()
                .any(|workstream| workstream.harness_kind == HarnessKind::BrokeredCloud)
        {
            reject(
                &mut rejections,
                "cloud_for_local_task",
                "product cloud agents cannot be used for repository-local work",
            );
        }

        let projected_single_cost_usd = request.root_model_rate.projected_cost(
            context_tokens.saturating_add(self.child_system_prompt_tokens),
            request.root_estimated_output_tokens,
        );
        let projected_multi_cost_usd = request
            .workstreams
            .iter()
            .map(|workstream| {
                workstream
                    .model_rate
                    .unwrap_or(request.root_model_rate)
                    .projected_cost(
                        workstream
                            .estimated_context_tokens
                            .saturating_add(self.child_system_prompt_tokens),
                        workstream.estimated_output_tokens,
                    )
            })
            .sum::<f64>();
        let single_token_work = context_tokens
            .saturating_add(self.child_system_prompt_tokens)
            .saturating_add(request.root_estimated_output_tokens);
        let multi_token_work = request.workstreams.iter().fold(0_u64, |total, workstream| {
            total
                .saturating_add(workstream.estimated_context_tokens)
                .saturating_add(self.child_system_prompt_tokens)
                .saturating_add(workstream.estimated_output_tokens)
        });
        let projected_weighted_tokens = request
            .workstreams
            .iter()
            .map(|workstream| {
                workstream
                    .estimated_context_tokens
                    .saturating_add(self.child_system_prompt_tokens) as f64
                    + workstream.estimated_output_tokens as f64 * self.output_token_weight
            })
            .sum::<f64>();
        if self
            .max_projected_weighted_tokens
            .is_some_and(|limit| projected_weighted_tokens > limit)
        {
            reject(
                &mut rejections,
                "projected_token_budget_exceeded",
                "the first parallel batch exceeds the shared weighted-token budget",
            );
        }
        let projected_cost_ratio = if projected_single_cost_usd > 0.0 {
            projected_multi_cost_usd / projected_single_cost_usd
        } else if projected_multi_cost_usd == 0.0 {
            multi_token_work as f64 / single_token_work.max(1) as f64
        } else {
            f64::INFINITY
        };
        if request.purpose != OrchestrationPurpose::ExternalResearch
            && projected_cost_ratio > self.max_projected_cost_ratio
        {
            reject(
                &mut rejections,
                "projected_cost_too_high",
                "fan-out cost exceeds the configured single-agent ratio",
            );
        }

        AdmissionDecision {
            admitted: rejections.is_empty(),
            rejections,
            projected_single_cost_usd,
            projected_multi_cost_usd,
            projected_cost_ratio,
            projected_weighted_tokens,
        }
    }
}

fn reject(rejections: &mut Vec<Rejection>, code: &str, detail: &str) {
    rejections.push(Rejection {
        code: code.to_string(),
        detail: detail.to_string(),
    });
}

fn overlapping_scope(workstreams: &[WorkstreamEstimate]) -> Option<String> {
    for (index, left) in workstreams.iter().enumerate() {
        for right in workstreams.iter().skip(index + 1) {
            for left_scope in &left.scopes {
                for right_scope in &right.scopes {
                    let left = normalize_scope(left_scope);
                    let right = normalize_scope(right_scope);
                    if left == right
                        || left.starts_with(&format!("{right}/"))
                        || right.starts_with(&format!("{left}/"))
                    {
                        return Some(format!("{left} <> {right}"));
                    }
                }
            }
        }
    }
    None
}

fn normalize_scope(scope: &str) -> String {
    scope.trim().trim_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(id: &str, scope: &str, tokens: u64) -> WorkstreamEstimate {
        WorkstreamEstimate {
            task_id: TaskId::new(id).unwrap(),
            scopes: BTreeSet::from([scope.to_string()]),
            estimated_context_tokens: tokens,
            estimated_output_tokens: 1_000,
            harness_kind: HarnessKind::Local,
            model: "strong".to_string(),
            model_rate: None,
        }
    }

    fn request() -> AdmissionRequest {
        AdmissionRequest {
            authorization: Authorization::UserRequested,
            purpose: OrchestrationPurpose::Explore,
            workstreams: vec![
                stream("api", "src/api", 25_000),
                stream("ui", "src/ui", 25_000),
            ],
            root_model: "strong".to_string(),
            root_model_rate: ModelRate {
                input_per_million_usd: 1.0,
                output_per_million_usd: 1.0,
            },
            root_estimated_output_tokens: 2_000,
            risk: RiskSignals::default(),
            external_research_required: false,
        }
    }

    #[test]
    fn admits_high_context_independent_fanout_with_bounded_cost() {
        let decision = AdmissionPolicy::default().evaluate(&request());
        assert!(decision.admitted, "{:?}", decision.rejections);
        assert!(decision.projected_cost_ratio <= 1.25);
    }

    #[test]
    fn rejects_overlap_small_context_and_unrequested_fanout() {
        let mut request = request();
        request.authorization = Authorization::None;
        request.workstreams[0].estimated_context_tokens = 1_000;
        request.workstreams[1].estimated_context_tokens = 1_000;
        request.workstreams[1].scopes = BTreeSet::from(["src/api/routes".to_string()]);
        let decision = AdmissionPolicy::default().evaluate(&request);
        let codes = decision
            .rejections
            .iter()
            .map(|rejection| rejection.code.as_str())
            .collect::<BTreeSet<_>>();
        assert!(codes.contains("authorization_required"));
        assert!(codes.contains("context_too_small"));
        assert!(codes.contains("overlapping_scopes"));
    }

    #[test]
    fn review_is_risk_triggered_and_cloud_is_external_only() {
        let mut request = request();
        request.purpose = OrchestrationPurpose::Review;
        request.workstreams.truncate(1);
        assert!(!AdmissionPolicy::default().evaluate(&request).admitted);
        request.risk.touches_auth_or_security = true;
        assert!(AdmissionPolicy::default().evaluate(&request).admitted);
        request.workstreams[0].harness_kind = HarnessKind::BrokeredCloud;
        assert!(!AdmissionPolicy::default().evaluate(&request).admitted);
    }

    #[test]
    fn absolute_token_budget_rejects_a_parallel_batch_before_spawn() {
        let policy = AdmissionPolicy {
            max_projected_weighted_tokens: Some(10_000.0),
            ..AdmissionPolicy::default()
        };
        let decision = policy.evaluate(&request());
        assert!(!decision.admitted);
        assert!(decision
            .rejections
            .iter()
            .any(|rejection| rejection.code == "projected_token_budget_exceeded"));
        assert!(decision.projected_weighted_tokens > 10_000.0);
    }
}
