use std::fmt;

use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};

use crate::tree::AccumulatorError;

const CONTEXT_TAG: &[u8] = b"scout-accumulator-context-v1";
const KEY_TAG: &[u8] = b"scout-accumulator-key-v1";

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest([u8; 32]);

impl Digest {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(self) -> String {
        let mut encoded = String::with_capacity(64);
        for byte in self.0 {
            use std::fmt::Write as _;
            write!(&mut encoded, "{byte:02x}").expect("writing to a string cannot fail");
        }
        encoded
    }

    pub fn from_hex(encoded: &str) -> Result<Self, AccumulatorError> {
        if encoded.len() != 64 {
            return Err(AccumulatorError::InvalidDigest);
        }
        let mut bytes = [0_u8; 32];
        for (index, pair) in encoded.as_bytes().chunks_exact(2).enumerate() {
            bytes[index] = (decode_nibble(pair[0])? << 4) | decode_nibble(pair[1])?;
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Digest({self})")
    }
}

impl Serialize for Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DigestVisitor;

        impl Visitor<'_> for DigestVisitor {
            type Value = Digest;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a 64-character hexadecimal SHA-256 digest")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Digest::from_hex(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(DigestVisitor)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccumulatorContext {
    domain: String,
    enterprise_id: String,
    namespace: String,
}

impl AccumulatorContext {
    pub fn new(
        domain: impl Into<String>,
        enterprise_id: impl Into<String>,
        namespace: impl Into<String>,
    ) -> Result<Self, AccumulatorError> {
        let context = Self {
            domain: domain.into(),
            enterprise_id: enterprise_id.into(),
            namespace: namespace.into(),
        };
        context.validate()?;
        Ok(context)
    }

    pub fn domain(&self) -> &str {
        &self.domain
    }

    pub fn enterprise_id(&self) -> &str {
        &self.enterprise_id
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn object_key(&self, object_id: &str) -> Digest {
        let mut hasher = Sha256::new();
        hasher.update(KEY_TAG);
        update_field(&mut hasher, self.domain.as_bytes());
        update_field(&mut hasher, self.enterprise_id.as_bytes());
        update_field(&mut hasher, self.namespace.as_bytes());
        update_field(&mut hasher, object_id.as_bytes());
        finish(hasher)
    }

    pub(crate) fn validate(&self) -> Result<(), AccumulatorError> {
        for (name, value) in [
            ("domain", self.domain.as_str()),
            ("enterprise_id", self.enterprise_id.as_str()),
            ("namespace", self.namespace.as_str()),
        ] {
            if value.is_empty() {
                return Err(AccumulatorError::EmptyContextField(name));
            }
        }
        Ok(())
    }

    pub(crate) fn digest(&self) -> Digest {
        let mut hasher = Sha256::new();
        hasher.update(CONTEXT_TAG);
        update_field(&mut hasher, self.domain.as_bytes());
        update_field(&mut hasher, self.enterprise_id.as_bytes());
        update_field(&mut hasher, self.namespace.as_bytes());
        finish(hasher)
    }
}

pub(crate) fn hash_tagged(tag: &[u8], parts: &[&[u8]]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(tag);
    for part in parts {
        hasher.update(part);
    }
    finish(hasher)
}

pub(crate) fn hash_tagged_with_field(tag: &[u8], fixed: &[&[u8]], field: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(tag);
    for part in fixed {
        hasher.update(part);
    }
    update_field(&mut hasher, field);
    finish(hasher)
}

fn update_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn finish(hasher: Sha256) -> Digest {
    Digest::from_bytes(hasher.finalize().into())
}

fn decode_nibble(byte: u8) -> Result<u8, AccumulatorError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(AccumulatorError::InvalidDigest),
    }
}
