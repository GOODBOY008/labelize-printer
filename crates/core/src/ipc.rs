use crate::*;
use anyhow::{Context, Result};
#[cfg(unix)]
use interprocess::local_socket::GenericFilePath;
#[cfg(windows)]
use interprocess::local_socket::GenericNamespaced;
use interprocess::local_socket::{prelude::*, ListenerOptions, Name, Stream};
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::path::PathBuf;

const MAX_MESSAGE: u64 = 4 * 1024 * 1024;

fn endpoint_id(paths: &AppPaths) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for b in paths.root.to_string_lossy().bytes() {
        hash = (hash ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    format!("labelize-printer-{hash:016x}")
}

#[cfg(unix)]
pub fn socket_path(paths: &AppPaths) -> PathBuf {
    // Keep macOS sockaddr_un below its 104-byte limit even with long home paths.
    std::env::temp_dir()
        .join(endpoint_id(paths))
        .join("ipc.sock")
}

pub fn endpoint(paths: &AppPaths) -> Result<Name<'static>> {
    #[cfg(unix)]
    {
        Ok(socket_path(paths)
            .to_fs_name::<GenericFilePath>()?
            .into_owned())
    }
    #[cfg(windows)]
    {
        Ok(endpoint_id(paths)
            .to_ns_name::<GenericNamespaced>()?
            .into_owned())
    }
}

pub fn listener(paths: &AppPaths) -> Result<interprocess::local_socket::Listener> {
    #[cfg(unix)]
    {
        let socket = socket_path(paths);
        crate::paths::private_dir(socket.parent().unwrap())?;
        // The caller holds the exclusive daemon lock before removing a stale socket.
        if socket.exists() {
            std::fs::remove_file(&socket)?;
        }
    }
    let options = ListenerOptions::new().name(endpoint(paths)?);
    #[cfg(windows)]
    let options = {
        use interprocess::os::windows::{
            local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
        };
        use std::os::windows::process::CommandExt;
        let result = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Security.Principal.WindowsIdentity]::GetCurrent().User.Value",
            ])
            .creation_flags(0x08000000)
            .output()?;
        anyhow::ensure!(
            result.status.success(),
            "Cannot determine the current user's SID"
        );
        let sid = String::from_utf8(result.stdout)?;
        let sid = sid.trim();
        anyhow::ensure!(
            sid.starts_with("S-1-")
                && sid
                    .chars()
                    .all(|c| c == 'S' || c == '-' || c.is_ascii_digit()),
            "Invalid user SID"
        );
        let sddl = widestring::U16CString::from_str(format!("D:P(A;;GA;;;{sid})"))?;
        options.security_descriptor(SecurityDescriptor::deserialize(&sddl)?)
    };
    let listener = options.create_sync()?;
    #[cfg(unix)]
    crate::paths::private_file(&socket_path(paths))?;
    Ok(listener)
}

pub fn request(paths: &AppPaths, request: Request) -> Result<Response> {
    let mut stream =
        Stream::connect(endpoint(paths)?).context("Cannot connect to the background service")?;
    write_json(
        &mut stream,
        &AuthenticatedRequest {
            token: paths.token()?,
            request,
        },
    )?;
    read_json(&mut BufReader::new(stream))
}

pub fn write_json(writer: &mut impl Write, value: &impl serde::Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_MESSAGE,
        "IPC response exceeds limit"
    );
    writer.write_all(&bytes)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

pub fn read_json<T: serde::de::DeserializeOwned>(reader: &mut impl BufRead) -> Result<T> {
    let mut bytes = Vec::new();
    reader.take(MAX_MESSAGE + 1).read_until(b'\n', &mut bytes)?;
    anyhow::ensure!(
        !bytes.is_empty() && bytes.len() as u64 <= MAX_MESSAGE && bytes.last() == Some(&b'\n'),
        "Invalid or oversized IPC message"
    );
    Ok(serde_json::from_slice(&bytes)?)
}
