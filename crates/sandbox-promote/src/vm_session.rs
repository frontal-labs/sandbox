//! A Tier 2 session: commands run in a guest VM, and its files live there. It is made by
//! promoting a Tier 1 session (see `promote_to_vm`). Snapshot and checkpoint are not built
//! for this tier yet, and they say so rather than fake a result.

use async_trait::async_trait;
use bytes::Bytes;
use sandbox_proto::v1::guest::guest_service_client::GuestServiceClient;
use sandbox_proto::v1::guest::{
    ListDirRequest, MkDirRequest, ReadFileRequest, RemoveRequest, RenameRequest, StatRequest,
    WriteFileRequest,
};
use sandbox_session::{Checkpoint, Session};
use sandbox_shell::{OutputChunk, OutputObserver};
use sandbox_types::hash::ContentHash;
use sandbox_types::{ErrorCode, ExecRequest, ExecResult, SandboxError, SessionId, Tier};
use sandbox_vfs::{DirEntry, FileHandle, FileStat, OpenFlags, SessionFs, VfsError, VfsResult};
use sandbox_vmm::QemuVm;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex as StdMutex;
use tonic::transport::Channel;
use tonic::Code;

use crate::promote::{promote_tree, PromotionReport};

/// Largest chunk one filesystem call moves, under the transport's message limit.
const CHUNK: usize = 1024 * 1024;

#[derive(Clone)]
struct OpenFile {
    path: String,
    readable: bool,
    writable: bool,
    append: bool,
}

/// The guest's filesystem, seen through the session filesystem trait. Each call is a
/// protocol message, so the guest's files are the only copy.
pub struct GuestFs {
    client: GuestServiceClient<Channel>,
    handles: StdMutex<HashMap<u64, OpenFile>>,
    next: AtomicU64,
}

impl GuestFs {
    pub fn new(client: GuestServiceClient<Channel>) -> Self {
        GuestFs {
            client,
            handles: StdMutex::new(HashMap::new()),
            next: AtomicU64::new(1),
        }
    }

    fn file(&self, handle: &FileHandle) -> VfsResult<OpenFile> {
        self.handles
            .lock()
            .map_err(|_| VfsError::Internal("handle table poisoned".into()))?
            .get(&handle.0)
            .cloned()
            .ok_or_else(|| VfsError::InvalidPath("the handle is closed".into()))
    }

    /// Writes `data` at `offset` in chunks. `truncate` empties the file first.
    async fn put(&self, path: &str, data: &[u8], offset: u64, truncate: bool) -> VfsResult<()> {
        let mut client = self.client.clone();
        let mut at = offset;
        let mut first = true;
        // An empty write still creates the file, and it may truncate.
        while first || at < offset + data.len() as u64 {
            let start = (at - offset) as usize;
            let end = (start + CHUNK).min(data.len());
            client
                .write_file(WriteFileRequest {
                    path: path.to_string(),
                    data: data[start..end].to_vec(),
                    offset: at,
                    truncate: truncate && first,
                })
                .await
                .map_err(|s| map_status(s, path))?;
            at += (end - start) as u64;
            first = false;
        }
        Ok(())
    }
}

fn map_status(status: tonic::Status, path: &str) -> VfsError {
    match status.code() {
        Code::NotFound => VfsError::NotFound(path.to_string()),
        Code::AlreadyExists => VfsError::AlreadyExists(path.to_string()),
        Code::InvalidArgument => VfsError::InvalidPath(status.message().to_string()),
        _ => VfsError::Internal(format!("the guest failed on {path}: {}", status.message())),
    }
}

fn unsupported(what: &str) -> VfsError {
    VfsError::Unsupported(format!("{what} is not available on a Tier 2 session yet"))
}

#[async_trait]
impl SessionFs for GuestFs {
    async fn total_bytes(&self) -> VfsResult<u64> {
        Err(unsupported("the total size"))
    }

    async fn open(&self, path: &Path, flags: OpenFlags) -> VfsResult<FileHandle> {
        let name = path.to_string_lossy().into_owned();
        let mut client = self.client.clone();
        let existing = client
            .stat(StatRequest { path: name.clone() })
            .await
            .ok()
            .map(|r| r.into_inner());
        match &existing {
            Some(stat) if stat.is_dir => {
                return Err(VfsError::InvalidPath(format!("{name} is a directory")))
            }
            Some(_) if flags.create && flags.exclusive => {
                return Err(VfsError::AlreadyExists(name.clone()))
            }
            None if !flags.create => return Err(VfsError::NotFound(name.clone())),
            None => self.put(&name, &[], 0, false).await?,
            Some(_) if flags.truncate => self.put(&name, &[], 0, true).await?,
            Some(_) => {}
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.handles
            .lock()
            .map_err(|_| VfsError::Internal("handle table poisoned".into()))?
            .insert(
                id,
                OpenFile {
                    path: name,
                    readable: flags.read || !flags.write,
                    writable: flags.write,
                    append: flags.append,
                },
            );
        Ok(FileHandle(id))
    }

    async fn read(&self, handle: &FileHandle, offset: u64, len: usize) -> VfsResult<Bytes> {
        let file = self.file(handle)?;
        if !file.readable {
            return Err(VfsError::PermissionDenied(format!(
                "{} is not open for reading",
                file.path
            )));
        }
        let mut client = self.client.clone();
        let mut out = Vec::new();
        let mut finished = false;
        while !finished && out.len() < len {
            let want = (len - out.len()).min(CHUNK);
            let chunk = client
                .read_file(ReadFileRequest {
                    path: file.path.clone(),
                    offset: offset + out.len() as u64,
                    max_bytes: want as u32,
                })
                .await
                .map_err(|s| map_status(s, &file.path))?
                .into_inner();
            finished = chunk.eof;
            out.extend_from_slice(&chunk.data);
        }
        Ok(Bytes::from(out))
    }

    async fn write(&self, handle: &FileHandle, offset: u64, data: &[u8]) -> VfsResult<usize> {
        let file = self.file(handle)?;
        if !file.writable {
            return Err(VfsError::PermissionDenied(format!(
                "{} is not open for writing",
                file.path
            )));
        }
        let at = if file.append {
            self.stat(Path::new(&file.path)).await?.size
        } else {
            offset
        };
        self.put(&file.path, data, at, false).await?;
        Ok(data.len())
    }

    async fn stat(&self, path: &Path) -> VfsResult<FileStat> {
        let name = path.to_string_lossy().into_owned();
        let mut client = self.client.clone();
        let stat = client
            .stat(StatRequest { path: name.clone() })
            .await
            .map_err(|s| map_status(s, &name))?
            .into_inner();
        Ok(FileStat {
            path: name,
            is_dir: stat.is_dir,
            is_file: stat.is_file,
            is_symlink: false,
            size: stat.size,
            mode: if stat.is_dir { 0o755 } else { 0o644 },
            modified: 0,
        })
    }

    async fn readdir(&self, path: &Path) -> VfsResult<Vec<DirEntry>> {
        let name = path.to_string_lossy().into_owned();
        let mut client = self.client.clone();
        let listing = client
            .list_dir(ListDirRequest { path: name.clone() })
            .await
            .map_err(|s| map_status(s, &name))?
            .into_inner();
        Ok(listing
            .entries
            .into_iter()
            .map(|e| DirEntry {
                name: e.name,
                is_dir: e.is_dir,
                is_file: !e.is_dir,
                is_symlink: false,
                size: e.size,
                mode: if e.is_dir { 0o755 } else { 0o644 },
            })
            .collect())
    }

    async fn mkdir(&self, path: &Path, _mode: u32) -> VfsResult<()> {
        let name = path.to_string_lossy().into_owned();
        self.client
            .clone()
            .mk_dir(MkDirRequest { path: name.clone() })
            .await
            .map_err(|s| map_status(s, &name))?;
        Ok(())
    }

    async fn rm(&self, path: &Path, recursive: bool) -> VfsResult<()> {
        let name = path.to_string_lossy().into_owned();
        self.client
            .clone()
            .remove(RemoveRequest {
                path: name.clone(),
                recursive,
            })
            .await
            .map_err(|s| map_status(s, &name))?;
        Ok(())
    }

    async fn rename(&self, from: &Path, to: &Path) -> VfsResult<()> {
        let from = from.to_string_lossy().into_owned();
        self.client
            .clone()
            .rename(RenameRequest {
                from: from.clone(),
                to: to.to_string_lossy().into_owned(),
            })
            .await
            .map_err(|s| map_status(s, &from))?;
        Ok(())
    }

    async fn symlink(&self, _target: &Path, _link: &Path) -> VfsResult<()> {
        Err(unsupported("symbolic links"))
    }

    async fn readlink(&self, _path: &Path) -> VfsResult<String> {
        Err(unsupported("symbolic links"))
    }

    async fn truncate(&self, path: &Path, size: u64) -> VfsResult<()> {
        let name = path.to_string_lossy().into_owned();
        let stat = self.stat(path).await?;
        if size > stat.size {
            return Err(unsupported("extending a file"));
        }
        // The protocol has no truncate call, so the kept bytes are rewritten.
        let kept = self.read_all(&name, size).await?;
        self.put(&name, &kept, 0, true).await
    }

    async fn watch(&self, _path: &Path) -> VfsResult<sandbox_vfs::Watch> {
        Err(unsupported("watching a directory"))
    }

    fn close(&self, handle: &FileHandle) {
        if let Ok(mut table) = self.handles.lock() {
            table.remove(&handle.0);
        }
    }

    async fn commit(&mut self) -> VfsResult<ContentHash> {
        Err(unsupported("committing the filesystem"))
    }
}

impl GuestFs {
    async fn read_all(&self, path: &str, size: u64) -> VfsResult<Vec<u8>> {
        let mut client = self.client.clone();
        let mut out = Vec::new();
        let mut finished = false;
        while !finished && (out.len() as u64) < size {
            let chunk = client
                .read_file(ReadFileRequest {
                    path: path.to_string(),
                    offset: out.len() as u64,
                    max_bytes: CHUNK as u32,
                })
                .await
                .map_err(|s| map_status(s, path))?
                .into_inner();
            finished = chunk.eof;
            out.extend_from_slice(&chunk.data);
        }
        out.truncate(size as usize);
        Ok(out)
    }
}

/// A Tier 2 session. The VM is stopped when the session is destroyed.
pub struct VmSession {
    id: SessionId,
    vm: tokio::sync::Mutex<Option<QemuVm>>,
    client: GuestServiceClient<Channel>,
    fs: GuestFs,
    random: AtomicU64,
}

impl VmSession {
    fn exec_error(error: tonic::Status) -> SandboxError {
        SandboxError::new(
            ErrorCode::VmBootFailed,
            format!("the guest refused the command: {}", error.message()),
        )
    }
}

#[async_trait]
impl Session for VmSession {
    fn id(&self) -> SessionId {
        self.id
    }

    fn tier(&self) -> Tier {
        Tier::Vm
    }

    async fn exec(&self, req: ExecRequest) -> Result<ExecResult, SandboxError> {
        if req.argv.is_empty() {
            return Err(SandboxError::invalid_argument("argv is empty"));
        }
        let reply = self
            .client
            .clone()
            .exec(sandbox_proto::v1::types::ExecRequest::from(req))
            .await
            .map_err(Self::exec_error)?
            .into_inner();
        ExecResult::try_from(reply)
            .map_err(|e| SandboxError::internal(format!("bad reply from the guest: {e}")))
    }

    async fn exec_streamed(
        &self,
        req: ExecRequest,
        observer: OutputObserver,
    ) -> Result<ExecResult, SandboxError> {
        // The guest returns a command's output when the command ends, so the observer sees
        // it in two chunks at the end, not as it is written. Finer streaming needs the
        // protocol to stream output.
        let result = self.exec(req).await?;
        if !result.stdout.is_empty() {
            observer(OutputChunk {
                fd: 1,
                data: result.stdout.clone(),
            });
        }
        if !result.stderr.is_empty() {
            observer(OutputChunk {
                fd: 2,
                data: result.stderr.clone(),
            });
        }
        Ok(result)
    }

    async fn run_script(&self, script: &str) -> Result<ExecResult, SandboxError> {
        self.exec(ExecRequest {
            argv: vec!["sh".into(), "-c".into(), script.into()],
            ..Default::default()
        })
        .await
    }

    fn fs(&self) -> &dyn SessionFs {
        &self.fs
    }

    async fn random_state(&self) -> u64 {
        self.random.load(Ordering::Relaxed)
    }

    async fn set_random_state(&self, state: u64) {
        self.random.store(state, Ordering::Relaxed);
    }

    async fn write_system_file(&self, path: &str, data: &[u8]) -> Result<(), SandboxError> {
        self.fs
            .put(path, data, 0, true)
            .await
            .map_err(|e| SandboxError::new(ErrorCode::VfsError, e.to_string()))
    }

    async fn snapshot(&self) -> Result<ContentHash, SandboxError> {
        Err(SandboxError::unsupported(
            "snapshots of Tier 2 sessions are not built yet",
        ))
    }

    async fn checkpoint(&self) -> Result<Checkpoint, SandboxError> {
        Err(SandboxError::unsupported(
            "checkpoints of Tier 2 sessions are not built yet",
        ))
    }

    async fn destroy(self: Box<Self>) -> Result<(), SandboxError> {
        match self.vm.into_inner() {
            Some(vm) => vm.stop().await,
            None => Ok(()),
        }
    }
}

/// Promotes a Tier 1 session into the running VM: its files are copied in, and the returned
/// session runs its commands in the guest. Processes do not survive the move.
pub async fn promote_to_vm(
    session: &dyn Session,
    vm: QemuVm,
) -> Result<(VmSession, PromotionReport), SandboxError> {
    let url = format!("http://{}", vm.agent());
    let channel = GuestServiceClient::connect(url.clone())
        .await
        .map_err(|e| {
            SandboxError::new(
                ErrorCode::VmBootFailed,
                format!("cannot reach the guest agent: {e}"),
            )
        })?;
    let mut client = channel.clone();
    let report = promote_tree(session.fs(), &mut client).await?;
    let promoted = VmSession {
        id: session.id(),
        vm: tokio::sync::Mutex::new(Some(vm)),
        client: channel.clone(),
        fs: GuestFs::new(channel),
        random: AtomicU64::new(session.random_state().await),
    };
    Ok((promoted, report))
}
