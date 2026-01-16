use serde::{Deserialize, Serialize};

/// Isolation level for transactions
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationLevel {
    #[default]
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

impl From<IsolationLevel> for crate::transaction::IsolationLevel {
    fn from(level: IsolationLevel) -> Self {
        match level {
            IsolationLevel::ReadCommitted => crate::transaction::IsolationLevel::ReadCommitted,
            IsolationLevel::RepeatableRead => crate::transaction::IsolationLevel::RepeatableRead,
            IsolationLevel::Serializable => crate::transaction::IsolationLevel::Serializable,
        }
    }
}
