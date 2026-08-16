use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Error, Result};

use super::code::{is_valid_dicom_uid, validate_text};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackingIdentity {
    id: String,
    uid: String,
}

impl TrackingIdentity {
    pub fn new(id: impl Into<String>, uid: impl Into<String>) -> Result<Self> {
        let id = id.into();
        let uid = uid.into();
        validate_text("tracking ID", &id, 10_240)?;
        if !is_valid_dicom_uid(&uid) {
            return Err(Error::InvalidInput(
                "tracking UID is not a valid DICOM UID".into(),
            ));
        }
        Ok(Self { id, uid })
    }

    pub fn generated(prefix: &str, ordinal: u64) -> Result<Self> {
        if ordinal == 0
            || prefix.is_empty()
            || prefix.len() > 12
            || !prefix.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(Error::InvalidInput(
                "tracking prefix must be 1..=12 ASCII letters or digits and ordinal must be positive"
                    .into(),
            ));
        }
        let id = format!("{prefix}-{ordinal:06}");
        let uid = format!("2.25.{}", Uuid::new_v4().as_u128());
        Self::new(id, uid)
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn uid(&self) -> &str {
        &self.uid
    }
}
