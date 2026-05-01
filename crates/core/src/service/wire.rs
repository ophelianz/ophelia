use super::*;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OpheliaWireCommand {
    pub id: u64,
    pub command: OpheliaCommand,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub(crate) enum OpheliaWireFrame {
    Response { id: u64, response: OpheliaResponse },
    Error { id: u64, error: OpheliaError },
    Event { event: OpheliaEvent },
}

pub(super) fn command_to_payload(command: &OpheliaWireCommand) -> Result<Vec<u8>, OpheliaError> {
    serde_json::to_vec(command).map_err(|error| OpheliaError::Transport {
        message: format!("failed to encode service command: {error}"),
    })
}

pub(super) fn frame_to_payload(frame: &OpheliaWireFrame) -> Result<Vec<u8>, OpheliaError> {
    serde_json::to_vec(frame).map_err(|error| OpheliaError::Transport {
        message: format!("failed to encode service frame: {error}"),
    })
}

pub(super) fn command_from_payload(payload: &[u8]) -> Result<OpheliaWireCommand, OpheliaError> {
    serde_json::from_slice(payload).map_err(|error| OpheliaError::BadRequest {
        message: format!("failed to parse service command: {error}"),
    })
}

pub(super) fn frame_from_payload(payload: &[u8]) -> Result<OpheliaWireFrame, OpheliaError> {
    serde_json::from_slice(payload).map_err(|error| OpheliaError::Transport {
        message: format!("failed to parse service frame: {error}"),
    })
}

pub(super) fn unexpected_wire_frame(expected: &str, frame: OpheliaWireFrame) -> OpheliaError {
    OpheliaError::Transport {
        message: format!("expected service {expected}, got {frame:?}"),
    }
}
