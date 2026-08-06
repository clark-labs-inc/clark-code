use std::fmt;

use serde::{Deserialize, Serialize};

use crate::hash::{hash_tagged, hash_tagged_with_field, AccumulatorContext, Digest};
use crate::proof::{Direction, Proof, ProofStep, ProofTerminal};

const EMPTY_ROOT_TAG: &[u8] = b"scout-accumulator-empty-root-v1";
const ROOT_TAG: &[u8] = b"scout-accumulator-root-v1";
const LEAF_TAG: &[u8] = b"scout-accumulator-leaf-v1";
const BRANCH_TAG: &[u8] = b"scout-accumulator-branch-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccumulatorError {
    EmptyContextField(&'static str),
    InvalidDigest,
    UnsupportedVersion,
    InvalidPartition,
    KeyCollision,
    CountOverflow,
    InvalidObjectId,
    MissingNode,
    Storage(String),
    InvalidProof(&'static str),
    RootMismatch,
}

impl fmt::Display for AccumulatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyContextField(field) => {
                write!(
                    formatter,
                    "accumulator context field {field} must not be empty"
                )
            }
            Self::InvalidDigest => formatter.write_str("invalid SHA-256 digest"),
            Self::UnsupportedVersion => {
                formatter.write_str("unsupported accumulator schema version")
            }
            Self::InvalidPartition => formatter.write_str("invalid accumulator partition"),
            Self::KeyCollision => {
                formatter.write_str("distinct object ids produced the same accumulator key")
            }
            Self::CountOverflow => formatter.write_str("accumulator subtree count overflow"),
            Self::InvalidObjectId => {
                formatter.write_str("accumulator object id must be 1..=4096 bytes without NUL")
            }
            Self::MissingNode => formatter.write_str("accumulator node is missing"),
            Self::Storage(error) => write!(formatter, "accumulator storage failed: {error}"),
            Self::InvalidProof(reason) => write!(formatter, "invalid accumulator proof: {reason}"),
            Self::RootMismatch => formatter.write_str("accumulator proof root mismatch"),
        }
    }
}

impl std::error::Error for AccumulatorError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertOutcome {
    Inserted,
    AlreadyPresent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccumulatorRoot {
    pub digest: Digest,
    pub count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubtreeCommitment {
    pub digest: Digest,
    pub count: u64,
    pub min_key: Digest,
    pub max_key: Digest,
}

impl SubtreeCommitment {
    pub(crate) fn leaf(key: Digest, object_id: &str) -> Self {
        Self {
            digest: hash_tagged_with_field(
                LEAF_TAG,
                &[key.as_bytes().as_slice()],
                object_id.as_bytes(),
            ),
            count: 1,
            min_key: key,
            max_key: key,
        }
    }

    pub(crate) fn branch(left: Self, right: Self) -> Result<(u16, Self), AccumulatorError> {
        validate_summary(&left)?;
        validate_summary(&right)?;
        if left.max_key >= right.min_key {
            return Err(AccumulatorError::InvalidProof(
                "branch child key ranges overlap or are reversed",
            ));
        }
        let min_key = left.min_key;
        let max_key = right.max_key;
        let bit = common_prefix_bits(min_key, max_key);
        if bit >= 256
            || bit_at(left.min_key, bit) != 0
            || bit_at(left.max_key, bit) != 0
            || bit_at(right.min_key, bit) != 1
            || bit_at(right.max_key, bit) != 1
        {
            return Err(AccumulatorError::InvalidProof(
                "branch is not a canonical binary radix split",
            ));
        }
        let count = left
            .count
            .checked_add(right.count)
            .ok_or(AccumulatorError::CountOverflow)?;
        let bit_bytes = bit.to_be_bytes();
        let left_count = left.count.to_be_bytes();
        let right_count = right.count.to_be_bytes();
        let digest = hash_tagged(
            BRANCH_TAG,
            &[
                &bit_bytes,
                left.digest.as_bytes(),
                &left_count,
                left.min_key.as_bytes(),
                left.max_key.as_bytes(),
                right.digest.as_bytes(),
                &right_count,
                right.min_key.as_bytes(),
                right.max_key.as_bytes(),
            ],
        );
        Ok((
            bit,
            Self {
                digest,
                count,
                min_key,
                max_key,
            },
        ))
    }
}

pub struct Accumulator {
    context: AccumulatorContext,
    tree: Option<Box<Node>>,
}

impl Accumulator {
    pub fn new(context: AccumulatorContext) -> Self {
        Self {
            context,
            tree: None,
        }
    }

    pub fn context(&self) -> &AccumulatorContext {
        &self.context
    }

    pub fn count(&self) -> u64 {
        self.tree.as_ref().map_or(0, |node| node.summary.count)
    }

    pub fn is_empty(&self) -> bool {
        self.tree.is_none()
    }

    pub fn root(&self) -> AccumulatorRoot {
        root_from_summary(&self.context, self.tree.as_ref().map(|node| node.summary))
    }

    pub fn contains(&self, object_id: &str) -> bool {
        if validate_object_id(object_id).is_err() {
            return false;
        }
        let key = self.context.object_key(object_id);
        self.terminal_leaf(key).is_some_and(|leaf| {
            let (leaf_key, leaf_object_id) = leaf.leaf_parts();
            leaf_key == key && leaf_object_id == object_id
        })
    }

    pub fn insert(
        &mut self,
        object_id: impl Into<String>,
    ) -> Result<InsertOutcome, AccumulatorError> {
        let object_id = object_id.into();
        validate_object_id(&object_id)?;
        let key = self.context.object_key(&object_id);
        if let Some(existing) = self.terminal_leaf(key) {
            let (existing_key, existing_object_id) = existing.leaf_parts();
            if existing_key == key {
                return if existing_object_id == object_id {
                    Ok(InsertOutcome::AlreadyPresent)
                } else {
                    Err(AccumulatorError::KeyCollision)
                };
            }
        }

        let leaf = Box::new(Node::leaf(key, object_id));
        self.tree = Some(match self.tree.take() {
            None => leaf,
            Some(root) => insert_new(root, leaf),
        });
        Ok(InsertOutcome::Inserted)
    }

    pub fn prove(&self, object_id: impl Into<String>) -> Proof {
        let query_object_id = object_id.into();
        let key = self.context.object_key(&query_object_id);
        let Some(mut node) = self.tree.as_deref() else {
            return Proof {
                context: self.context.clone(),
                object_id: query_object_id,
                terminal: ProofTerminal::Empty,
                steps: Vec::new(),
            };
        };
        let mut steps = Vec::new();
        loop {
            match &node.kind {
                NodeKind::Leaf {
                    object_id: terminal_object_id,
                    ..
                } => {
                    return Proof {
                        context: self.context.clone(),
                        object_id: query_object_id,
                        terminal: ProofTerminal::Leaf {
                            object_id: terminal_object_id.clone(),
                        },
                        steps,
                    };
                }
                NodeKind::Branch { bit, left, right } => {
                    let direction = Direction::from_bit(bit_at(key, *bit));
                    let (next, sibling) = match direction {
                        Direction::Left => (left.as_ref(), right.summary),
                        Direction::Right => (right.as_ref(), left.summary),
                    };
                    steps.push(ProofStep {
                        branch_bit: *bit,
                        direction,
                        sibling,
                    });
                    node = next;
                }
            }
        }
    }

    fn terminal_leaf(&self, key: Digest) -> Option<&Node> {
        let mut node = self.tree.as_deref()?;
        loop {
            match &node.kind {
                NodeKind::Leaf { .. } => return Some(node),
                NodeKind::Branch { bit, left, right } => {
                    node = if bit_at(key, *bit) == 0 { left } else { right };
                }
            }
        }
    }
}

struct Node {
    kind: NodeKind,
    summary: SubtreeCommitment,
}

enum NodeKind {
    Leaf {
        key: Digest,
        object_id: String,
    },
    Branch {
        bit: u16,
        left: Box<Node>,
        right: Box<Node>,
    },
}

impl Node {
    fn leaf(key: Digest, object_id: String) -> Self {
        Self {
            summary: SubtreeCommitment::leaf(key, &object_id),
            kind: NodeKind::Leaf { key, object_id },
        }
    }

    fn branch(left: Box<Self>, right: Box<Self>) -> Self {
        let (bit, summary) = SubtreeCommitment::branch(left.summary, right.summary)
            .expect("insertion only constructs canonical disjoint branches");
        Self {
            kind: NodeKind::Branch { bit, left, right },
            summary,
        }
    }

    fn leaf_parts(&self) -> (Digest, &str) {
        match &self.kind {
            NodeKind::Leaf { key, object_id } => (*key, object_id),
            NodeKind::Branch { .. } => unreachable!("terminal traversal must end at a leaf"),
        }
    }
}

fn insert_new(root: Box<Node>, leaf: Box<Node>) -> Box<Node> {
    let leaf_key = leaf.summary.min_key;
    let terminal_key = find_terminal_key(&root, leaf_key);
    let split_bit = common_prefix_bits(leaf_key, terminal_key);
    insert_at(root, leaf, split_bit)
}

fn find_terminal_key(node: &Node, key: Digest) -> Digest {
    let mut current = node;
    loop {
        match &current.kind {
            NodeKind::Leaf { key, .. } => return *key,
            NodeKind::Branch { bit, left, right } => {
                current = if bit_at(key, *bit) == 0 { left } else { right };
            }
        }
    }
}

fn insert_at(root: Box<Node>, leaf: Box<Node>, split_bit: u16) -> Box<Node> {
    match root.kind {
        NodeKind::Branch { bit, left, right } if bit < split_bit => {
            if bit_at(leaf.summary.min_key, bit) == 0 {
                Box::new(Node::branch(insert_at(left, leaf, split_bit), right))
            } else {
                Box::new(Node::branch(left, insert_at(right, leaf, split_bit)))
            }
        }
        _ => {
            if bit_at(leaf.summary.min_key, split_bit) == 0 {
                Box::new(Node::branch(leaf, root))
            } else {
                Box::new(Node::branch(root, leaf))
            }
        }
    }
}

pub(crate) fn root_from_summary(
    context: &AccumulatorContext,
    summary: Option<SubtreeCommitment>,
) -> AccumulatorRoot {
    let context_digest = context.digest();
    match summary {
        None => AccumulatorRoot {
            digest: hash_tagged(EMPTY_ROOT_TAG, &[context_digest.as_bytes()]),
            count: 0,
        },
        Some(summary) => {
            let count = summary.count.to_be_bytes();
            AccumulatorRoot {
                digest: hash_tagged(
                    ROOT_TAG,
                    &[
                        context_digest.as_bytes(),
                        summary.digest.as_bytes(),
                        &count,
                        summary.min_key.as_bytes(),
                        summary.max_key.as_bytes(),
                    ],
                ),
                count: summary.count,
            }
        }
    }
}

pub(crate) fn bit_at(key: Digest, bit: u16) -> u8 {
    let byte = key.as_bytes()[usize::from(bit / 8)];
    (byte >> (7 - (bit % 8))) & 1
}

pub(crate) fn common_prefix_bits(left: Digest, right: Digest) -> u16 {
    for (index, (left_byte, right_byte)) in left
        .as_bytes()
        .iter()
        .zip(right.as_bytes().iter())
        .enumerate()
    {
        let difference = left_byte ^ right_byte;
        if difference != 0 {
            return (index as u16) * 8 + difference.leading_zeros() as u16;
        }
    }
    256
}

pub(crate) fn validate_summary(summary: &SubtreeCommitment) -> Result<(), AccumulatorError> {
    if summary.count == 0 {
        return Err(AccumulatorError::InvalidProof(
            "non-empty subtree has a zero count",
        ));
    }
    if summary.min_key > summary.max_key {
        return Err(AccumulatorError::InvalidProof(
            "subtree key range is reversed",
        ));
    }
    Ok(())
}

pub(crate) fn validate_object_id(object_id: &str) -> Result<(), AccumulatorError> {
    if object_id.is_empty() || object_id.len() > 4096 || object_id.contains('\0') {
        return Err(AccumulatorError::InvalidObjectId);
    }
    Ok(())
}
