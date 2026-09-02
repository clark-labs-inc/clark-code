use super::*;
use provider_local::{ToolPack, ToolRegistry};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

#[derive(Default)]
struct Control {
    epoch: AtomicU64,
    deny: AtomicBool,
    read_revoked: AtomicBool,
    text_changed: AtomicBool,
}

struct Fake(Arc<Control>);

impl Integration for Fake {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "fake".into(),
            name: "Fake".into(),
            description: "Test fixture".into(),
            capabilities: vec!["selected_text".into()],
            experimental: true,
        }
    }

    fn availability(&self) -> Availability {
        Availability {
            supported: true,
            detail: String::new(),
        }
    }

    fn epoch(&self) -> u64 {
        self.0.epoch.load(Ordering::SeqCst)
    }

    fn interactive(&self) -> bool {
        true
    }

    fn approve_read(&self, _: &str) -> bool {
        !self.0.deny.load(Ordering::SeqCst)
    }

    fn conversations(&self) -> Result<Vec<Conversation>, String> {
        Ok(vec![Conversation {
            id: "self-chat".into(),
            self_address: "self@example.test".into(),
        }])
    }

    fn read(&self, _: &Conversation) -> Result<Vec<Message>, String> {
        if self.0.read_revoked.load(Ordering::SeqCst) {
            return Err("OS access revoked".into());
        }
        Ok(vec![
            Message {
                id: "1".into(),
                text: if self.0.text_changed.load(Ordering::SeqCst) {
                    "edited".into()
                } else {
                    "first".into()
                },
                from_me: true,
                unix_seconds: 1,
            },
            Message {
                id: "2".into(),
                text: "second".into(),
                from_me: true,
                unix_seconds: 2,
            },
        ])
    }
}

fn scope() -> Scope {
    Scope {
        owner: "account-a".into(),
        task: "task-a".into(),
        generation: 1,
        instance: 1,
    }
}

fn registry(control: &Arc<Control>) -> Registry {
    let mut registry = Registry::new();
    registry.register(Box::new(Fake(control.clone()))).unwrap();
    registry
}

fn connect(registry: &mut Registry, scope: &Scope) {
    registry.connect(scope, "fake").unwrap();
    registry.select(scope, "fake", "self-chat").unwrap();
}

#[test]
fn registration_is_explicit_and_rejects_duplicate_ids() {
    let control = Arc::new(Control::default());
    let mut registry = registry(&control);
    assert_eq!(registry.catalog().len(), 1);
    assert!(registry.register(Box::new(Fake(control))).is_err());
    assert!(registry.availability("not-installed").is_err());
}

#[test]
fn selection_must_be_explicitly_enabled_before_the_tool_can_read() {
    let mut registry = registry(&Arc::default());
    connect(&mut registry, &scope());
    assert!(registry.read_tool(&scope(), "fake").is_err());
    assert_eq!(
        registry
            .enable_read_tool(&scope(), "fake", vec!["2".into()])
            .unwrap(),
        1
    );
    assert_eq!(registry.read_tool(&scope(), "fake").unwrap(), "second");
}

#[test]
fn task_account_generation_and_session_instance_are_all_authoritative() {
    let mut registry = registry(&Arc::default());
    connect(&mut registry, &scope());
    registry
        .enable_read_tool(&scope(), "fake", vec!["1".into()])
        .unwrap();
    for foreign in [
        Scope {
            task: "task-b".into(),
            ..scope()
        },
        Scope {
            owner: "account-b".into(),
            ..scope()
        },
        Scope {
            generation: 2,
            ..scope()
        },
        Scope {
            instance: 2,
            ..scope()
        },
    ] {
        assert!(registry.read_tool(&foreign, "fake").is_err());
    }
}

#[test]
fn revoke_sleep_lock_and_changed_text_fail_closed() {
    let control = Arc::new(Control::default());
    let mut registry = registry(&control);
    connect(&mut registry, &scope());
    registry
        .enable_read_tool(&scope(), "fake", vec!["1".into()])
        .unwrap();
    control.epoch.fetch_add(1, Ordering::SeqCst);
    assert!(registry.read_tool(&scope(), "fake").is_err());

    connect(&mut registry, &scope());
    registry
        .enable_read_tool(&scope(), "fake", vec!["1".into()])
        .unwrap();
    control.text_changed.store(true, Ordering::SeqCst);
    assert!(registry.read_tool(&scope(), "fake").is_err());

    control.text_changed.store(false, Ordering::SeqCst);
    connect(&mut registry, &scope());
    registry
        .enable_read_tool(&scope(), "fake", vec!["1".into()])
        .unwrap();
    control.read_revoked.store(true, Ordering::SeqCst);
    assert!(registry.read_tool(&scope(), "fake").is_err());

    control.read_revoked.store(false, Ordering::SeqCst);
    registry.revoke("fake");
    assert!(registry.read_tool(&scope(), "fake").is_err());
}

#[test]
fn reselection_clears_prior_tool_access_and_selection_is_bounded() {
    let mut registry = registry(&Arc::default());
    connect(&mut registry, &scope());
    registry
        .enable_read_tool(&scope(), "fake", vec!["1".into()])
        .unwrap();
    registry.select(&scope(), "fake", "self-chat").unwrap();
    assert!(registry.read_tool(&scope(), "fake").is_err());
    assert!(registry
        .enable_read_tool(&scope(), "fake", vec!["unseen".into()])
        .is_err());
    assert!(registry
        .enable_read_tool(&scope(), "fake", vec!["1".into(); 21])
        .is_err());
    assert!(registry
        .enable_read_tool(&scope(), "fake", vec!["1".into(), "1".into()])
        .is_err());
}

#[test]
fn tool_pack_registers_only_the_fixed_argument_free_read_tool() {
    let shared = Arc::new(Mutex::new(None));
    let pack = ReadToolPack::new(shared);
    let mut tools = ToolRegistry::new(None);
    pack.install(&mut tools).unwrap();
    let tool = tools.get("read_imessage_selection").unwrap();
    assert_eq!(tool.parameters()["additionalProperties"], false);
    assert!(!tool.mutating());
    assert!(tools.get("send_imessage").is_none());
}
