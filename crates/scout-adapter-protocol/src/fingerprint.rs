use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{ProtocolError, ProtocolResult};

pub(crate) fn canonical_sha256(value: &impl Serialize) -> ProtocolResult<String> {
    let encoded = serde_json::to_vec(value)
        .map_err(|error| ProtocolError::Serialization(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}
