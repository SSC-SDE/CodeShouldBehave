//! Serde codecs reuse the existing shared identity constructors and validators.
use crate::{RepositoryId, ScopeVersion, SecurityDomain, SnapshotId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepositoryWire {
    security_domain: String,
    provider: String,
    owner: String,
    name: String,
}
impl From<&RepositoryId> for RepositoryWire {
    fn from(value: &RepositoryId) -> Self {
        Self {
            security_domain: value.security_domain().as_str().into(),
            provider: value.provider().into(),
            owner: value.owner().into(),
            name: value.name().into(),
        }
    }
}
impl RepositoryWire {
    fn into_shared(self) -> Result<RepositoryId, String> {
        RepositoryId::new(
            SecurityDomain::new(self.security_domain)?,
            self.provider,
            self.owner,
            self.name,
        )
    }
}
pub(crate) fn repository_from_value(value: &serde_json::Value) -> Result<RepositoryId, String> {
    serde_json::from_value::<RepositoryWire>(value.clone())
        .map_err(|_| "Invalid design repository".to_string())?
        .into_shared()
}
pub(crate) mod repository {
    use super::*;
    pub fn serialize<S: Serializer>(
        value: &RepositoryId,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        RepositoryWire::from(value).serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<RepositoryId, D::Error> {
        RepositoryWire::deserialize(deserializer)?
            .into_shared()
            .map_err(serde::de::Error::custom)
    }
}
pub(crate) mod snapshot {
    use super::*;
    pub fn serialize<S: Serializer>(value: &SnapshotId, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(value.as_str())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SnapshotId, D::Error> {
        SnapshotId::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
pub(crate) mod scope_version {
    use super::*;
    pub fn serialize<S: Serializer>(
        value: &ScopeVersion,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(value.as_str())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<ScopeVersion, D::Error> {
        ScopeVersion::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
