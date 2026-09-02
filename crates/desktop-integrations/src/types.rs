use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub experimental: bool,
}

/// Constructed by the native host after authorizing the live session. Never
/// deserialize an owner or account generation supplied by a renderer/model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub owner: String,
    pub task: String,
    pub generation: u64,
    pub instance: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Availability {
    pub supported: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Conversation {
    pub id: String,
    pub self_address: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Message {
    pub id: String,
    pub text: String,
    pub from_me: bool,
    pub unix_seconds: i64,
}

/// A compiled, trusted adapter. There is no dynamic code loading or model-
/// writable manifest. New adapters register here, not in the coding tool list.
/// Platform dialogs MUST be executed on the application's main thread.
pub trait Integration: Send {
    fn manifest(&self) -> Manifest;
    fn availability(&self) -> Availability;
    fn epoch(&self) -> u64;
    fn interactive(&self) -> bool;
    fn approve_read(&self, task: &str) -> bool;
    fn conversations(&self) -> Result<Vec<Conversation>, String>;
    fn read(&self, conversation: &Conversation) -> Result<Vec<Message>, String>;
}
