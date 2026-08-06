# Clark planning comparative judge contract v2

The model owns every semantic judgment. The host may verify identities, exact
ordered schemas, enum membership, complete membership of declared behavior and
evidence IDs, and whole-response lifecycle. It must never infer, relabel,
normalize, merge, or fill a semantic field.

Evaluation has three model-owned stages:

1. Judge each immutable frozen plan exactly once without executor outcomes.
2. Compare the plan-delivered and plan-discarded executions together in one
   blinded packet that shares the already-judged frozen plan.
3. Adjudicate the accepted plan and pair verdicts across context treatments.

Every candidate verdict receives an independent Qwen audit. The audit reads the
same source packet and the complete candidate. It rejects internal
contradictions, incorrect packet claims, invalid causal attribution, and claims
unsupported by cited evidence. Rejection discards the whole candidate and asks
the judge model to generate a new verdict using the critique. No host repair is
permitted.

Plans and paired arms are identified by cryptographic receipts. The host fails
closed unless each matched pair has identical plan, planner context, retrieval,
and planner trajectory bytes; exactly one arm received the complete plan; and
the delivered plan hash and length equal the stored plan hash and length.

Hidden checks are factual evidence, not scores. Tool-call counts, keyword
counts, checklist completion, and citation counts are not quality proxies.
Equivalent correct implementations count. Judge knowledge for operational
influence, not mere assignment, availability, or citation.

Generate local evidence judgments before aggregate conclusions. Return only
the exact ordered JSON schema requested for the current stage. Cite precise
packet locations. Do not expose private chain-of-thought.
