use serde::{Deserialize, Serialize};

use crate::hash::{AccumulatorContext, Digest};
use crate::proof::{Direction, Proof, ProofStep, ProofTerminal};
use crate::tree::{
    bit_at, common_prefix_bits, root_from_summary, validate_object_id, validate_summary,
    AccumulatorError, AccumulatorRoot, InsertOutcome, SubtreeCommitment,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoredNode {
    Leaf {
        key: Digest,
        object_id: String,
    },
    Branch {
        branch_bit: u16,
        left: SubtreeCommitment,
        right: SubtreeCommitment,
    },
}

impl StoredNode {
    pub fn commitment(
        &self,
        context: &AccumulatorContext,
    ) -> Result<SubtreeCommitment, AccumulatorError> {
        match self {
            Self::Leaf { key, object_id } => {
                validate_object_id(object_id)?;
                if context.object_key(object_id) != *key {
                    return Err(AccumulatorError::InvalidProof(
                        "stored leaf key does not match its context and object id",
                    ));
                }
                Ok(SubtreeCommitment::leaf(*key, object_id))
            }
            Self::Branch {
                branch_bit,
                left,
                right,
            } => {
                let (observed_bit, commitment) = SubtreeCommitment::branch(*left, *right)?;
                if observed_bit != *branch_bit {
                    return Err(AccumulatorError::InvalidProof(
                        "stored branch bit is not its canonical radix split",
                    ));
                }
                Ok(commitment)
            }
        }
    }

    pub fn digest(&self, context: &AccumulatorContext) -> Result<Digest, AccumulatorError> {
        Ok(self.commitment(context)?.digest)
    }

    fn leaf(context: &AccumulatorContext, object_id: String) -> Self {
        Self::Leaf {
            key: context.object_key(&object_id),
            object_id,
        }
    }

    fn branch(left: SubtreeCommitment, right: SubtreeCommitment) -> Result<Self, AccumulatorError> {
        let (branch_bit, _) = SubtreeCommitment::branch(left, right)?;
        Ok(Self::Branch {
            branch_bit,
            left,
            right,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccumulatorHead {
    pub root: AccumulatorRoot,
    pub summary: Option<SubtreeCommitment>,
}

impl AccumulatorHead {
    pub fn empty(context: &AccumulatorContext) -> Self {
        Self {
            root: root_from_summary(context, None),
            summary: None,
        }
    }

    pub fn from_summary(context: &AccumulatorContext, summary: Option<SubtreeCommitment>) -> Self {
        Self {
            root: root_from_summary(context, summary),
            summary,
        }
    }

    pub fn validate(&self, context: &AccumulatorContext) -> Result<(), AccumulatorError> {
        if let Some(summary) = self.summary {
            validate_summary(&summary)?;
        }
        if self.root != root_from_summary(context, self.summary) {
            return Err(AccumulatorError::RootMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccumulatorMutation {
    pub previous: AccumulatorHead,
    pub next: AccumulatorHead,
    pub outcome: InsertOutcome,
    pub nodes: Vec<StoredNode>,
    pub obsolete_nodes: Vec<Digest>,
}

pub fn plan_insert(
    context: &AccumulatorContext,
    head: AccumulatorHead,
    object_id: impl Into<String>,
    mut read_node: impl FnMut(Digest) -> Result<Option<StoredNode>, AccumulatorError>,
) -> Result<AccumulatorMutation, AccumulatorError> {
    context.validate()?;
    head.validate(context)?;
    let object_id = object_id.into();
    validate_object_id(&object_id)?;
    let new_leaf = StoredNode::leaf(context, object_id.clone());
    let new_leaf_summary = new_leaf.commitment(context)?;
    let Some(root_summary) = head.summary else {
        let next = AccumulatorHead::from_summary(context, Some(new_leaf_summary));
        return Ok(AccumulatorMutation {
            previous: head,
            next,
            outcome: InsertOutcome::Inserted,
            nodes: vec![new_leaf],
            obsolete_nodes: Vec::new(),
        });
    };
    let (terminal, frames) = load_terminal(
        context,
        root_summary,
        new_leaf_summary.min_key,
        &mut read_node,
    )?;
    let (terminal_key, terminal_object_id) = match &terminal {
        StoredNode::Leaf { key, object_id } => (*key, object_id),
        StoredNode::Branch { .. } => {
            return Err(AccumulatorError::InvalidProof(
                "persistent traversal did not end at a leaf",
            ))
        }
    };
    if terminal_key == new_leaf_summary.min_key {
        return if terminal_object_id == &object_id {
            Ok(AccumulatorMutation {
                previous: head,
                next: head,
                outcome: InsertOutcome::AlreadyPresent,
                nodes: Vec::new(),
                obsolete_nodes: Vec::new(),
            })
        } else {
            Err(AccumulatorError::KeyCollision)
        };
    }

    let split_bit = common_prefix_bits(new_leaf_summary.min_key, terminal_key);
    let insertion_index = frames
        .iter()
        .position(|frame| frame.branch_bit > split_bit)
        .unwrap_or(frames.len());
    let existing = frames
        .get(insertion_index)
        .map_or(terminal.commitment(context)?, |frame| frame.parent);
    let split = if bit_at(new_leaf_summary.min_key, split_bit) == 0 {
        StoredNode::branch(new_leaf_summary, existing)?
    } else {
        StoredNode::branch(existing, new_leaf_summary)?
    };
    let mut current = split.commitment(context)?;
    let mut nodes = vec![new_leaf, split];
    for frame in frames[..insertion_index].iter().rev() {
        let parent = match frame.direction {
            Direction::Left => StoredNode::branch(current, frame.sibling)?,
            Direction::Right => StoredNode::branch(frame.sibling, current)?,
        };
        current = parent.commitment(context)?;
        nodes.push(parent);
    }
    let next = AccumulatorHead::from_summary(context, Some(current));
    let obsolete_nodes = frames[..insertion_index]
        .iter()
        .map(|frame| frame.parent.digest)
        .collect();
    Ok(AccumulatorMutation {
        previous: head,
        next,
        outcome: InsertOutcome::Inserted,
        nodes,
        obsolete_nodes,
    })
}

pub fn prove_persistent(
    context: &AccumulatorContext,
    head: AccumulatorHead,
    object_id: impl Into<String>,
    mut read_node: impl FnMut(Digest) -> Result<Option<StoredNode>, AccumulatorError>,
) -> Result<Proof, AccumulatorError> {
    context.validate()?;
    head.validate(context)?;
    let object_id = object_id.into();
    if object_id.len() > 4096 || object_id.contains('\0') {
        return Err(AccumulatorError::InvalidObjectId);
    }
    let Some(root_summary) = head.summary else {
        return Ok(Proof {
            context: context.clone(),
            object_id,
            terminal: ProofTerminal::Empty,
            steps: Vec::new(),
        });
    };
    let (terminal, frames) = load_terminal(
        context,
        root_summary,
        context.object_key(&object_id),
        &mut read_node,
    )?;
    let StoredNode::Leaf {
        object_id: terminal_object_id,
        ..
    } = terminal
    else {
        return Err(AccumulatorError::InvalidProof(
            "persistent traversal did not end at a leaf",
        ));
    };
    Ok(Proof {
        context: context.clone(),
        object_id,
        terminal: ProofTerminal::Leaf {
            object_id: terminal_object_id,
        },
        steps: frames
            .into_iter()
            .map(|frame| ProofStep {
                branch_bit: frame.branch_bit,
                direction: frame.direction,
                sibling: frame.sibling,
            })
            .collect(),
    })
}

#[derive(Clone, Copy)]
struct PathFrame {
    branch_bit: u16,
    direction: Direction,
    sibling: SubtreeCommitment,
    parent: SubtreeCommitment,
}

fn load_terminal(
    context: &AccumulatorContext,
    root_summary: SubtreeCommitment,
    key: Digest,
    read_node: &mut impl FnMut(Digest) -> Result<Option<StoredNode>, AccumulatorError>,
) -> Result<(StoredNode, Vec<PathFrame>), AccumulatorError> {
    let mut expected = root_summary;
    let mut frames = Vec::new();
    loop {
        let node = read_node(expected.digest)?.ok_or(AccumulatorError::MissingNode)?;
        if node.commitment(context)? != expected {
            return Err(AccumulatorError::InvalidProof(
                "stored node does not match its parent commitment",
            ));
        }
        match node {
            StoredNode::Leaf { .. } => return Ok((node, frames)),
            StoredNode::Branch {
                branch_bit,
                left,
                right,
            } => {
                let direction = Direction::from_bit(bit_at(key, branch_bit));
                let (child, sibling) = match direction {
                    Direction::Left => (left, right),
                    Direction::Right => (right, left),
                };
                frames.push(PathFrame {
                    branch_bit,
                    direction,
                    sibling,
                    parent: expected,
                });
                expected = child;
            }
        }
    }
}
