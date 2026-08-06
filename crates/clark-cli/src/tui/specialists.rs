use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpecialistContinuityKind {
    Scout,
    Security,
    Scientist,
    Rsi,
}

impl SpecialistContinuityKind {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match name {
            "scout" => Some(Self::Scout),
            "security" => Some(Self::Security),
            "scientist" => Some(Self::Scientist),
            "rsi" => Some(Self::Rsi),
            _ => None,
        }
    }

    pub(crate) fn continuity_owner(self) -> &'static str {
        match self {
            Self::Scout => "authoritative Clark Scout workspace API",
            Self::Security => "sealed Clark Security cloud sync",
            Self::Scientist | Self::Rsi => "verified Clark science artifact sync",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CloudContinuityReceipt {
    pub(crate) scope_id: String,
    pub(crate) file_count: usize,
    pub(crate) verified_segment_count: usize,
    pub(crate) total_bytes: u64,
}

impl CloudContinuityReceipt {
    pub(crate) fn from_projection(payload: &Value) -> Result<Option<Self>, String> {
        let Some(value) = payload.get("cloudSync") else {
            return Ok(None);
        };
        let object = value
            .as_object()
            .ok_or("Clark specialist cloud receipt is not an object")?;
        let expected = [
            "scope_id",
            "file_count",
            "verified_segment_count",
            "total_bytes",
        ];
        if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
            return Err("Clark specialist cloud receipt fields are incomplete or unknown".into());
        }
        let scope_id = object["scope_id"]
            .as_str()
            .ok_or("Clark specialist cloud receipt scope is not text")?
            .to_string();
        let file_count = usize::try_from(
            object["file_count"]
                .as_u64()
                .ok_or("Clark specialist cloud receipt file count is invalid")?,
        )
        .map_err(|_| "Clark specialist cloud receipt file count is too large")?;
        let verified_segment_count = usize::try_from(
            object["verified_segment_count"]
                .as_u64()
                .ok_or("Clark specialist cloud receipt segment count is invalid")?,
        )
        .map_err(|_| "Clark specialist cloud receipt segment count is too large")?;
        let total_bytes = object["total_bytes"]
            .as_u64()
            .ok_or("Clark specialist cloud receipt byte count is invalid")?;
        let receipt = Self {
            scope_id,
            file_count,
            verified_segment_count,
            total_bytes,
        };
        if receipt.scope_id.is_empty()
            || receipt.scope_id.len() > 128
            || !receipt.scope_id.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
        {
            return Err("Clark specialist cloud receipt has an invalid scope identity".into());
        }
        Ok(Some(receipt))
    }

    pub(crate) fn required_from_projection(payload: &Value) -> Result<Self, String> {
        Self::from_projection(payload)?.ok_or_else(|| {
            "Clark specialist completion omitted its required cloud synchronization receipt".into()
        })
    }

    pub(crate) fn summary(&self) -> String {
        format!(
            "Clark Cloud verified specialist journals and artifacts · {} files · {} segments · {} bytes · scope {}",
            self.file_count, self.verified_segment_count, self.total_bytes, self.scope_id
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CloudContinuity {
    last_receipt: Option<CloudContinuityReceipt>,
}

impl CloudContinuity {
    pub(crate) fn apply_projection(&mut self, payload: &Value) -> Result<Option<String>, String> {
        let Some(receipt) = CloudContinuityReceipt::from_projection(payload)? else {
            return Ok(None);
        };
        let summary = receipt.summary();
        self.last_receipt = Some(receipt);
        Ok(Some(summary))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_four_paid_specialists_have_an_explicit_cloud_owner() {
        let owners = ["scout", "security", "scientist", "rsi"]
            .into_iter()
            .map(|name| {
                SpecialistContinuityKind::from_name(name)
                    .expect("known specialist")
                    .continuity_owner()
            })
            .collect::<Vec<_>>();
        assert_eq!(owners.len(), 4);
        assert!(owners.iter().all(|owner| owner.contains("Clark")));
    }

    #[test]
    fn verified_worker_receipt_remains_visible_and_typed() {
        let payload = serde_json::json!({
            "cloudSync": {
                "scope_id": "specialist-session-7",
                "file_count": 3,
                "verified_segment_count": 5,
                "total_bytes": 1200
            }
        });
        let mut continuity = CloudContinuity::default();
        let summary = continuity
            .apply_projection(&payload)
            .expect("valid receipt")
            .expect("receipt present");
        assert!(summary.contains("3 files"));
        assert!(summary.contains("5 segments"));
        assert!(summary.contains("specialist-session-7"));
    }

    #[test]
    fn malformed_or_ambiguous_receipts_fail_closed() {
        let malformed = serde_json::json!({
            "cloudSync": {
                "scope_id": "../escape",
                "file_count": 1,
                "verified_segment_count": 1,
                "total_bytes": 4
            }
        });
        assert!(CloudContinuityReceipt::from_projection(&malformed).is_err());
        assert_eq!(
            CloudContinuityReceipt::from_projection(&serde_json::json!({})).unwrap(),
            None
        );
        assert!(CloudContinuityReceipt::required_from_projection(&serde_json::json!({})).is_err());
    }
}
