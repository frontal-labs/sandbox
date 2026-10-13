use sandbox_types::policy::ResourceLimits;

pub fn check_memory_limit(limits: &ResourceLimits, requested: u64) -> Result<(), sandbox_types::SandboxError> {
    if let Some(max) = limits.memory_bytes {
        if requested > max {
            return Err(sandbox_types::SandboxError::resource_exhausted(
                format!("Memory limit exceeded: requested {} bytes, limit {} bytes", requested, max)
            ));
        }
    }
    Ok(())
}

pub fn check_cpu_limit(limits: &ResourceLimits, requested_ms: u64) -> Result<(), sandbox_types::SandboxError> {
    if let Some(max) = limits.cpu_ms_per_sec {
        if requested_ms > max {
            return Err(sandbox_types::SandboxError::resource_exhausted(
                format!("CPU limit exceeded: requested {} ms, limit {} ms", requested_ms, max)
            ));
        }
    }
    Ok(())
}

pub fn check_file_size(limits: &ResourceLimits, size: u64) -> Result<(), sandbox_types::SandboxError> {
    if let Some(max) = limits.max_file_size {
        if size > max {
            return Err(sandbox_types::SandboxError::resource_exhausted(
                format!("File size limit exceeded: {} bytes, limit {} bytes", size, max)
            ));
        }
    }
    Ok(())
}

pub fn check_total_size(limits: &ResourceLimits, size: u64) -> Result<(), sandbox_types::SandboxError> {
    if let Some(max) = limits.max_total_size {
        if size > max {
            return Err(sandbox_types::SandboxError::resource_exhausted(
                format!("Total size limit exceeded: {} bytes, limit {} bytes", size, max)
            ));
        }
    }
    Ok(())
}

pub fn check_inodes(limits: &ResourceLimits, count: u64) -> Result<(), sandbox_types::SandboxError> {
    if let Some(max) = limits.max_inodes {
        if count > max {
            return Err(sandbox_types::SandboxError::resource_exhausted(
                format!("Inode limit exceeded: {}, limit {}", count, max)
            ));
        }
    }
    Ok(())
}

pub fn check_path_depth(limits: &ResourceLimits, depth: u32) -> Result<(), sandbox_types::SandboxError> {
    if let Some(max) = limits.max_path_depth {
        if depth > max {
            return Err(sandbox_types::SandboxError::resource_exhausted(
                format!("Path depth limit exceeded: {}, limit {}", depth, max)
            ));
        }
    }
    Ok(())
}