use crate::hash::{AccumulatorContext, Digest};
use crate::proof::Direction;
use crate::tree::{bit_at, common_prefix_bits, validate_object_id, AccumulatorError};

use super::proof::{MapProof, MapProofStep, MapProofTerminal};
use super::{MapHead, MapStoredNode, MapSubtreeCommitment};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapMutationOutcome {
    Inserted,
    Updated,
    Unchanged,
    Removed,
    Absent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapMutation {
    pub previous: MapHead,
    pub next: MapHead,
    pub outcome: MapMutationOutcome,
    pub nodes: Vec<MapStoredNode>,
    /// Nodes no longer reachable from `next`.
    ///
    /// These are only garbage-collection candidates. Callers retaining older
    /// heads must keep their nodes, and should never delete a candidate before
    /// storing all new nodes and performing reachability or reference checks.
    pub gc_candidates: Vec<Digest>,
}

pub fn plan_map_put(
    context: &AccumulatorContext,
    head: MapHead,
    object_id: impl Into<String>,
    value_digest: Digest,
    mut read_node: impl FnMut(Digest) -> Result<Option<MapStoredNode>, AccumulatorError>,
) -> Result<MapMutation, AccumulatorError> {
    context.validate()?;
    head.validate(context)?;
    let object_id = object_id.into();
    validate_object_id(&object_id)?;
    let new_leaf = MapStoredNode::leaf(context, object_id.clone(), value_digest);
    let new_summary = new_leaf.commitment(context)?;
    let Some(root_summary) = head.summary else {
        return Ok(MapMutation {
            previous: head,
            next: MapHead::from_summary(context, Some(new_summary)),
            outcome: MapMutationOutcome::Inserted,
            nodes: vec![new_leaf],
            gc_candidates: Vec::new(),
        });
    };
    let (terminal, frames) =
        load_terminal(context, root_summary, new_summary.min_key, &mut read_node)?;
    let (
        MapStoredNode::Leaf {
            key: terminal_key,
            object_id: terminal_id,
            value_digest: terminal_value,
        },
        terminal_summary,
    ) = (&terminal, terminal.commitment(context)?)
    else {
        return Err(AccumulatorError::InvalidProof(
            "map traversal did not end at a leaf",
        ));
    };

    if *terminal_key == new_summary.min_key {
        if terminal_id != &object_id {
            return Err(AccumulatorError::KeyCollision);
        }
        if *terminal_value == value_digest {
            return Ok(MapMutation {
                previous: head,
                next: head,
                outcome: MapMutationOutcome::Unchanged,
                nodes: Vec::new(),
                gc_candidates: Vec::new(),
            });
        }
        let (summary, mut nodes) = rebuild_path(context, new_summary, &frames)?;
        nodes.insert(0, new_leaf);
        let mut gc_candidates = Vec::with_capacity(frames.len() + 1);
        gc_candidates.push(terminal_summary.digest);
        gc_candidates.extend(frames.iter().map(|frame| frame.parent.digest));
        return Ok(MapMutation {
            previous: head,
            next: MapHead::from_summary(context, Some(summary)),
            outcome: MapMutationOutcome::Updated,
            nodes,
            gc_candidates,
        });
    }

    let split_bit = common_prefix_bits(new_summary.min_key, *terminal_key);
    let insertion_index = frames
        .iter()
        .position(|frame| frame.branch_bit > split_bit)
        .unwrap_or(frames.len());
    let existing = frames
        .get(insertion_index)
        .map_or(terminal_summary, |frame| frame.parent);
    let split = if bit_at(new_summary.min_key, split_bit) == 0 {
        MapStoredNode::branch(new_summary, existing)?
    } else {
        MapStoredNode::branch(existing, new_summary)?
    };
    let mut current = split.commitment(context)?;
    let mut nodes = vec![new_leaf, split];
    for frame in frames[..insertion_index].iter().rev() {
        let parent = frame.parent_with(current)?;
        current = parent.commitment(context)?;
        nodes.push(parent);
    }
    Ok(MapMutation {
        previous: head,
        next: MapHead::from_summary(context, Some(current)),
        outcome: MapMutationOutcome::Inserted,
        nodes,
        gc_candidates: frames[..insertion_index]
            .iter()
            .map(|frame| frame.parent.digest)
            .collect(),
    })
}

pub fn plan_map_remove(
    context: &AccumulatorContext,
    head: MapHead,
    object_id: impl Into<String>,
    mut read_node: impl FnMut(Digest) -> Result<Option<MapStoredNode>, AccumulatorError>,
) -> Result<MapMutation, AccumulatorError> {
    context.validate()?;
    head.validate(context)?;
    let object_id = object_id.into();
    validate_object_id(&object_id)?;
    let Some(root_summary) = head.summary else {
        return Ok(no_change(head, MapMutationOutcome::Absent));
    };
    let key = context.object_key(&object_id);
    let (terminal, frames) = load_terminal(context, root_summary, key, &mut read_node)?;
    let MapStoredNode::Leaf {
        key: terminal_key,
        object_id: terminal_id,
        ..
    } = &terminal
    else {
        return Err(AccumulatorError::InvalidProof(
            "map traversal did not end at a leaf",
        ));
    };
    if *terminal_key != key {
        return Ok(no_change(head, MapMutationOutcome::Absent));
    }
    if terminal_id != &object_id {
        return Err(AccumulatorError::KeyCollision);
    }

    let terminal_digest = terminal.digest(context)?;
    if frames.is_empty() {
        return Ok(MapMutation {
            previous: head,
            next: MapHead::empty(context),
            outcome: MapMutationOutcome::Removed,
            nodes: Vec::new(),
            gc_candidates: vec![terminal_digest],
        });
    }
    let retained_frames = &frames[..frames.len() - 1];
    let promoted = frames
        .last()
        .expect("a non-empty path has a final frame")
        .sibling;
    let (summary, nodes) = rebuild_path(context, promoted, retained_frames)?;
    let mut gc_candidates = Vec::with_capacity(frames.len() + 1);
    gc_candidates.push(terminal_digest);
    gc_candidates.extend(frames.iter().map(|frame| frame.parent.digest));
    Ok(MapMutation {
        previous: head,
        next: MapHead::from_summary(context, Some(summary)),
        outcome: MapMutationOutcome::Removed,
        nodes,
        gc_candidates,
    })
}

pub fn prove_map_persistent(
    context: &AccumulatorContext,
    head: MapHead,
    object_id: impl Into<String>,
    mut read_node: impl FnMut(Digest) -> Result<Option<MapStoredNode>, AccumulatorError>,
) -> Result<MapProof, AccumulatorError> {
    context.validate()?;
    head.validate(context)?;
    let object_id = object_id.into();
    if object_id.len() > 4096 || object_id.contains('\0') {
        return Err(AccumulatorError::InvalidObjectId);
    }
    let Some(root_summary) = head.summary else {
        return Ok(MapProof {
            context: context.clone(),
            object_id,
            terminal: MapProofTerminal::Empty,
            steps: Vec::new(),
        });
    };
    let (terminal, frames) = load_terminal(
        context,
        root_summary,
        context.object_key(&object_id),
        &mut read_node,
    )?;
    let MapStoredNode::Leaf {
        object_id: terminal_id,
        value_digest,
        ..
    } = terminal
    else {
        return Err(AccumulatorError::InvalidProof(
            "map traversal did not end at a leaf",
        ));
    };
    Ok(MapProof {
        context: context.clone(),
        object_id,
        terminal: MapProofTerminal::Leaf {
            object_id: terminal_id,
            value_digest,
        },
        steps: frames
            .into_iter()
            .map(|frame| MapProofStep {
                branch_bit: frame.branch_bit,
                direction: frame.direction,
                sibling: frame.sibling,
            })
            .collect(),
    })
}

fn no_change(head: MapHead, outcome: MapMutationOutcome) -> MapMutation {
    MapMutation {
        previous: head,
        next: head,
        outcome,
        nodes: Vec::new(),
        gc_candidates: Vec::new(),
    }
}

fn rebuild_path(
    context: &AccumulatorContext,
    mut current: MapSubtreeCommitment,
    frames: &[PathFrame],
) -> Result<(MapSubtreeCommitment, Vec<MapStoredNode>), AccumulatorError> {
    let mut nodes = Vec::with_capacity(frames.len());
    for frame in frames.iter().rev() {
        let parent = frame.parent_with(current)?;
        current = parent.commitment(context)?;
        nodes.push(parent);
    }
    Ok((current, nodes))
}

#[derive(Clone, Copy)]
struct PathFrame {
    branch_bit: u16,
    direction: Direction,
    sibling: MapSubtreeCommitment,
    parent: MapSubtreeCommitment,
}

impl PathFrame {
    fn parent_with(self, child: MapSubtreeCommitment) -> Result<MapStoredNode, AccumulatorError> {
        match self.direction {
            Direction::Left => MapStoredNode::branch(child, self.sibling),
            Direction::Right => MapStoredNode::branch(self.sibling, child),
        }
    }
}

fn load_terminal(
    context: &AccumulatorContext,
    root_summary: MapSubtreeCommitment,
    key: Digest,
    read_node: &mut impl FnMut(Digest) -> Result<Option<MapStoredNode>, AccumulatorError>,
) -> Result<(MapStoredNode, Vec<PathFrame>), AccumulatorError> {
    let mut expected = root_summary;
    let mut frames = Vec::new();
    loop {
        let node = read_node(expected.digest)?.ok_or(AccumulatorError::MissingNode)?;
        if node.commitment(context)? != expected {
            return Err(AccumulatorError::InvalidProof(
                "stored map node does not match its parent commitment",
            ));
        }
        match node {
            MapStoredNode::Leaf { .. } => return Ok((node, frames)),
            MapStoredNode::Branch {
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
