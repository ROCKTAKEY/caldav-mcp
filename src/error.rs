use serde::Serialize;
#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    pub code: String,
    pub message: String,
    #[serde(flatten)]
    pub details: serde_json::Map<String, serde_json::Value>,
}
pub type Result<T> = std::result::Result<T, Error>;
impl Error {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: Default::default(),
        }
    }
    pub fn input(message: impl Into<String>) -> Self {
        Self::new("invalid_input", message)
    }
    pub fn detail(mut self, key: &str, value: impl Serialize) -> Self {
        self.details.insert(
            key.into(),
            serde_json::to_value(value).expect("serializable detail"),
        );
        self
    }
}
