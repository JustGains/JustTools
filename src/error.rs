use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub struct ToolError {
    tool: String,
    message: String,
    exit_code: i32,
}

impl ToolError {
    pub fn new(tool: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            message: message.into(),
            exit_code: 1,
        }
    }

    pub fn usage(tool: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            message: message.into(),
            exit_code: 2,
        }
    }

    pub fn cancelled(tool: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            message: "cancelled".into(),
            exit_code: 130,
        }
    }

    pub fn with_code(tool: impl Into<String>, message: impl Into<String>, exit_code: i32) -> Self {
        Self {
            tool: tool.into(),
            message: message.into(),
            exit_code,
        }
    }

    pub fn tool(&self) -> &str {
        &self.tool
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl Display for ToolError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ToolError {}

pub type ToolResult<T = ()> = Result<T, ToolError>;
