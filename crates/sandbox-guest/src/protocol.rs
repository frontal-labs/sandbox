use sandbox_types::SandboxError;
use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct ExecRequest {
    #[prost(string, repeated, tag = "1")]
    pub argv: Vec<String>,
    
    #[prost(string, repeated, tag = "2")]
    pub env: Vec<String>,
    
    #[prost(string, tag = "3")]
    pub cwd: String,
    
    #[prost(bytes, tag = "4")]
    pub stdin: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ExecResponse {
    #[prost(int32, tag = "1")]
    pub exit_code: i32,
    
    #[prost(bytes, tag = "2")]
    pub stdout: Vec<u8>,
    
    #[prost(bytes, tag = "3")]
    pub stderr: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct FileRequest {
    #[prost(string, tag = "1")]
    pub path: String,
    
    #[prost(int32, tag = "2")]
    pub operation: i32,
    
    #[prost(bytes, tag = "3")]
    pub data: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct FileResponse {
    #[prost(bytes, tag = "1")]
    pub data: Vec<u8>,
    
    #[prost(string, tag = "2")]
    pub error: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum FileOperation {
    Read = 0,
    Write = 1,
    List = 2,
    Stat = 3,
}

pub fn encode_message<T: Message>(msg: &T) -> Result<Vec<u8>, SandboxError> {
    let mut buf = Vec::new();
    msg.encode(&mut buf).map_err(|e| SandboxError::internal(format!("Encoding failed: {}", e)))?;
    Ok(buf)
}

pub fn decode_message<T: Message + Default>(data: &[u8]) -> Result<T, SandboxError> {
    T::decode(data).map_err(|e| SandboxError::internal(format!("Decoding failed: {}", e)))
}