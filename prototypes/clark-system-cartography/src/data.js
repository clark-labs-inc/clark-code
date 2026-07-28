export const summary = {
  services: 1248,
  repositories: 184,
  cloudScopes: 43,
  machines: 26,
  journeys: 9,
  mapped: 87,
  gaps: 36,
  denied: 12,
  unreachable: 3,
  secrets: 0,
};

export const journeys = [
  { id: "order", name: "Order to Cash", progress: 91, mapped: 214, gaps: 6, denied: 1, unreachable: 0, updated: "10:58:13 AM" },
  { id: "procure", name: "Procure to Pay", progress: 84, mapped: 163, gaps: 8, denied: 2, unreachable: 0, updated: "10:58:11 AM" },
  { id: "record", name: "Record to Report", progress: 89, mapped: 198, gaps: 5, denied: 1, unreachable: 0, updated: "10:58:12 AM" },
  { id: "hire", name: "Hire to Retire", progress: 86, mapped: 142, gaps: 4, denied: 1, unreachable: 1, updated: "10:57:58 AM" },
  { id: "support", name: "Service to Support", progress: 82, mapped: 167, gaps: 6, denied: 2, unreachable: 1, updated: "10:57:50 AM" },
  { id: "release", name: "Design to Release", progress: 90, mapped: 186, gaps: 3, denied: 1, unreachable: 0, updated: "10:58:15 AM" },
  { id: "contract", name: "Source to Contract", progress: 88, mapped: 121, gaps: 2, denied: 1, unreachable: 0, updated: "10:57:59 AM" },
  { id: "operations", name: "IT Operations", progress: 93, mapped: 113, gaps: 0, denied: 1, unreachable: 1, updated: "10:58:14 AM" },
  { id: "insight", name: "Data to Insight", progress: 90, mapped: 144, gaps: 2, denied: 2, unreachable: 1, updated: "10:58:07 AM" },
];

export const workstreams = [
  { id: "aws", source: "AWS Organizations", task: "AWS production hierarchy", machine: "scout-aws-02", state: "scanning", progress: 68, evidence: 2431, updated: "10:58:15 AM" },
  { id: "github", source: "GitHub Enterprise", task: "Organization inventory", machine: "scout-gh-01", state: "verified", progress: 100, evidence: 1842, updated: "10:57:54 AM" },
  { id: "gcp", source: "GCP Projects", task: "Project and service inventory", machine: "scout-gcp-01", state: "scanning", progress: 54, evidence: 1201, updated: "10:58:12 AM" },
  { id: "okta", source: "Okta", task: "Directory and groups", machine: "scout-okta-01", state: "verified", progress: 100, evidence: 812, updated: "10:57:47 AM" },
  { id: "k8s", source: "Kubernetes", task: "Cluster and workload inventory", machine: "scout-k8s-01", state: "scanning", progress: 72, evidence: 3118, updated: "10:58:10 AM" },
  { id: "snowflake", source: "Snowflake", task: "Account and object inventory", machine: "scout-sf-01", state: "verified", progress: 100, evidence: 1007, updated: "10:57:41 AM" },
  { id: "datadog", source: "Datadog", task: "Monitors and integrations", machine: "scout-dd-01", state: "denied", progress: 0, evidence: 0, updated: "10:55:22 AM" },
  { id: "pagerduty", source: "PagerDuty", task: "Services and schedules", machine: "scout-pd-01", state: "verified", progress: 100, evidence: 423, updated: "10:57:35 AM" },
  { id: "dns", source: "DNS (Route 53)", task: "Hosted zones and records", machine: "scout-dns-01", state: "scanning", progress: 37, evidence: 456, updated: "10:58:08 AM" },
  { id: "jenkins", source: "CI/CD (Jenkins)", task: "Jobs and pipelines", machine: "scout-jenkins-01", state: "unreachable", progress: 0, evidence: 0, updated: "10:54:02 AM" },
  { id: "catalog", source: "Service Catalog", task: "Internal service catalog", machine: "scout-catalog-01", state: "resumable", progress: 48, evidence: 640, updated: "10:56:17 AM" },
];

export const evidenceRows = [
  { id: "config", artifact: "Checkout API DB config", detail: "application.yml", source: "GitHub Enterprise", collector: "clark-collector-aws-1", hash: "a1b2c3d4…", tier: "T2", classification: "Internal", observed: "10:41 AM", status: "verified" },
  { id: "terraform", artifact: "Aurora cluster module", detail: "main.tf", source: "Terraform Cloud", collector: "clark-terraform-ingestor", hash: "b2c3d4e5…", tier: "T2", classification: "Internal", observed: "10:36 AM", status: "verified" },
  { id: "build", artifact: "CI build artifact", detail: "checkout-svc:1.28.7", source: "AWS CodeBuild", collector: "clark-codebuild-1", hash: "c3d4e5f6…", tier: "T2", classification: "Internal", observed: "10:33 AM", status: "verified" },
  { id: "task", artifact: "ECS task definition", detail: "checkout-svc:38", source: "Amazon ECS", collector: "clark-ecs-collector", hash: "d4e5f6a7…", tier: "T2", classification: "Internal", observed: "10:28 AM", status: "verified" },
  { id: "iam", artifact: "Task role permissions", detail: "ecsTaskExecutionRole", source: "AWS IAM", collector: "clark-iam-collector", hash: "e5f6a7b8…", tier: "T2", classification: "Internal", observed: "10:25 AM", status: "verified" },
  { id: "aurora", artifact: "Aurora connection", detail: "pg_stat_activity sample", source: "Amazon Aurora", collector: "clark-aurora-sensor-1", hash: "f6a7b8c9…", tier: "T1", classification: "Confidential", observed: "10:21 AM", status: "verified" },
  { id: "trace", artifact: "Live trace: write order", detail: "trace_id 892713f3e2", source: "Datadog APM", collector: "clark-datadog-sensor", hash: "a7b8c9d0…", tier: "T1", classification: "Confidential", observed: "10:18 AM", status: "verified" },
  { id: "superseded", artifact: "ECS task definition (old)", detail: "checkout-svc:37", source: "Amazon ECS", collector: "clark-ecs-collector", hash: "9f8e7d6c…", tier: "T2", classification: "Internal", observed: "Jul 25, 11:12 PM", status: "superseded" },
];

export const provenance = [
  { label: "GitHub Enterprise", detail: "acme/checkout-svc" },
  { label: "Terraform Cloud", detail: "checkout-svc" },
  { label: "AWS CodeBuild", detail: "build #21435" },
  { label: "Amazon ECS", detail: "prod-blue" },
  { label: "AWS IAM", detail: "execution role" },
  { label: "Amazon Aurora", detail: "orders-prod" },
  { label: "Datadog APM", detail: "checkout-svc" },
  { label: "PagerDuty", detail: "Orders" },
  { label: "Order to Cash", detail: "Capture order" },
];

export const graphNodes = [
  { id: "journey", label: "Order to Cash", type: "Journey", x: 50, y: 230, state: "verified" },
  { id: "web", label: "Web app", type: "Application", x: 280, y: 80, state: "verified" },
  { id: "checkout", label: "Checkout API", type: "Service", x: 280, y: 230, state: "verified" },
  { id: "stripe", label: "Stripe", type: "Vendor", x: 520, y: 80, state: "verified" },
  { id: "orders", label: "Orders service", type: "Service", x: 520, y: 230, state: "verified" },
  { id: "events", label: "EventBridge", type: "Event bus", x: 520, y: 390, state: "verified" },
  { id: "warehouse", label: "Warehouse service", type: "Service", x: 770, y: 230, state: "partial" },
  { id: "aurora", label: "Aurora orders", type: "Database", x: 770, y: 390, state: "verified" },
  { id: "carrier", label: "Carrier webhook", type: "External API", x: 1010, y: 160, state: "gap" },
  { id: "notify", label: "Customer notifications", type: "Service", x: 1010, y: 330, state: "verified" },
];

export const graphEdges = [
  ["journey", "checkout", "implements"],
  ["web", "checkout", "calls"],
  ["checkout", "stripe", "calls"],
  ["checkout", "orders", "calls"],
  ["orders", "events", "publishes"],
  ["events", "warehouse", "triggers"],
  ["orders", "aurora", "writes"],
  ["warehouse", "carrier", "calls"],
  ["warehouse", "notify", "publishes"],
];

export const journeyStages = [
  { id: "storefront", number: 1, title: "Storefront", subtitle: "Browse & add to cart", system: "Web app", cloud: "AWS · us-east-1", state: "verified" },
  { id: "checkout", number: 2, title: "Checkout", subtitle: "Create order", system: "Checkout API", cloud: "AWS · us-east-1", state: "verified" },
  { id: "payment", number: 3, title: "Payment", subtitle: "Authorize & capture", system: "Stripe", cloud: "Global", state: "verified" },
  { id: "fulfillment", number: 4, title: "Fulfillment", subtitle: "Warehouse fulfills order", system: "Orders service", cloud: "GCP · us-central1", state: "partial" },
  { id: "finance", number: 5, title: "Finance", subtitle: "Invoice & record revenue", system: "Snowflake", cloud: "GCP · us-central1", state: "verified" },
];

export const scenarios = [
  { id: "happy", name: "Happy path", purpose: "Successful order-to-cash flow", events: 18, injection: "—", assertions: 12, ready: true },
  { id: "decline", name: "Payment decline", purpose: "Card declined at authorization", events: 18, injection: "Stripe: card_declined", assertions: 11, ready: true },
  { id: "duplicate", name: "Duplicate event", purpose: "Idempotency on duplicate webhook", events: 18, injection: "EventBridge: duplicate", assertions: 10, ready: true },
  { id: "outage", name: "Regional warehouse outage", purpose: "Warehouse service unavailable", events: 17, injection: "Warehouse: 503", assertions: 11, ready: true },
  { id: "inventory", name: "Stale inventory", purpose: "Insufficient inventory at pick", events: 18, injection: "Warehouse: stale_inventory", assertions: 10, ready: true },
  { id: "notify", name: "Notification failure", purpose: "Customer notification failure", events: 18, injection: "SendGrid: 5xx", assertions: 9, ready: true },
];

export const machines = [
  { id: "m-7f3c2a8d", name: "scout-aws-02", platform: "Linux", architecture: "x86_64", location: "AWS us-east-1", version: "1.8.0", state: "active", heartbeat: "12 sec ago", tasks: 3 },
  { id: "m-95a3bc11", name: "scout-gh-01", platform: "Linux", architecture: "arm64", location: "GitHub Actions", version: "1.8.0", state: "active", heartbeat: "18 sec ago", tasks: 1 },
  { id: "m-d182c4aa", name: "scout-gcp-01", platform: "Linux", architecture: "x86_64", location: "GCP us-central1", version: "1.8.0", state: "active", heartbeat: "7 sec ago", tasks: 2 },
  { id: "m-aa7e9120", name: "stan-macbook", platform: "macOS", architecture: "arm64", location: "San Francisco", version: "1.8.0", state: "active", heartbeat: "24 sec ago", tasks: 0 },
  { id: "m-197fed12", name: "scl-build-host", platform: "Linux", architecture: "x86_64", location: "SCL datacenter", version: "1.7.4", state: "upgrade", heartbeat: "2 min ago", tasks: 1 },
  { id: "m-02f39a81", name: "windows-qa-arm", platform: "Windows", architecture: "arm64", location: "UTM QA", version: "1.8.0", state: "offline", heartbeat: "4 hrs ago", tasks: 0 },
];

export const sources = [
  { id: "github", name: "GitHub Enterprise", namespace: "github.com/acme-corp", kind: "Source forge", state: "connected", coverage: "184 repositories", last: "28 min ago" },
  { id: "aws", name: "AWS Organizations", namespace: "o-a1b2c3d4", kind: "Cloud", state: "connected", coverage: "31 accounts · 17 regions", last: "12 min ago" },
  { id: "gcp", name: "Google Cloud", namespace: "organizations/81720341", kind: "Cloud", state: "connected", coverage: "12 projects · 4 folders", last: "34 min ago" },
  { id: "okta", name: "Okta", namespace: "acme.okta.com", kind: "Identity", state: "connected", coverage: "4,281 users · 327 groups", last: "41 min ago" },
  { id: "datadog", name: "Datadog", namespace: "acme.datadoghq.com", kind: "Observability", state: "attention", coverage: "Scope denied: integrations", last: "2 hrs ago" },
  { id: "snowflake", name: "Snowflake", namespace: "acme-org", kind: "Data", state: "connected", coverage: "86 databases · 1,904 schemas", last: "1 hr ago" },
  { id: "pagerduty", name: "PagerDuty", namespace: "acme.pagerduty.com", kind: "Incident response", state: "connected", coverage: "423 services", last: "1 hr ago" },
  { id: "catalog", name: "Internal service catalog", namespace: "catalog.acme.internal", kind: "Ownership", state: "unreachable", coverage: "Last complete: 3 days ago", last: "3 days ago" },
];

export const auditEvents = [
  { action: "Charter v12 issued", actor: "Sarah Graham", detail: "Added EU production and carrier ownership requirements", time: "Jul 26, 10:02 AM" },
  { action: "Machine enrolled", actor: "Clark Root CA", detail: "scout-aws-02 · Linux x86_64", time: "Jul 26, 9:44 AM" },
  { action: "Claim adjudicated", actor: "Alex Morgan", detail: "Checkout API writes orders to Aurora production", time: "Jul 26, 9:12 AM" },
  { action: "Machine revoked", actor: "Priya Shah", detail: "legacy-jenkins-scanner", time: "Jul 25, 4:31 PM" },
  { action: "Classification policy changed", actor: "Security automation", detail: "Cloud identity observations elevated to Restricted", time: "Jul 25, 1:18 PM" },
];
