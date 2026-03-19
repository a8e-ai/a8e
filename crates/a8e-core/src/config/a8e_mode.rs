use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum A8eMode {
    Auto,
    Approve,
    SmartApprove,
    Chat,
}

impl FromStr for A8eMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(A8eMode::Auto),
            "approve" => Ok(A8eMode::Approve),
            "smart_approve" => Ok(A8eMode::SmartApprove),
            "chat" => Ok(A8eMode::Chat),
            _ => Err(format!("invalid mode: {}", s)),
        }
    }
}
