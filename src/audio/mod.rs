pub mod decode;
pub mod resample;
pub mod transport;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("Audio file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Audio decoding failed: {0}")]
    Decode(#[from] symphonia::core::errors::Error),
    #[error("Audio work cancelled")]
    Cancelled,
    #[error("{0}")]
    Invalid(String),
}

pub type AudioResult<T> = std::result::Result<T, AudioError>;
pub type Stereo = [f32; 2];
