#[path = "../../../src-tauri/src/commands/security_cloud/client.rs"]
mod client;
// The imported evidence pipeline mirrors the server's signed receipt shape;
// keeping those bindings explicit is clearer than wrapping them solely for a
// lint threshold in this small headless adapter crate.
#[allow(clippy::too_many_arguments)]
#[path = "../../../src-tauri/src/commands/security_cloud/evidence.rs"]
mod evidence;
#[path = "../../../src-tauri/src/commands/security_cloud/identity.rs"]
mod identity;
#[path = "../../../src-tauri/src/commands/security_cloud/ingest.rs"]
mod ingest;
#[path = "../../../src-tauri/src/commands/security_cloud/model.rs"]
mod model;
#[path = "../../../src-tauri/src/commands/security_cloud/poc.rs"]
mod poc;

pub use ingest::{sync_security_scans, SecuritySyncRequest};
pub use model::{SecurityCloudScanSync, SecurityCloudScanSyncStatus, SecurityCloudSyncResult};
