#[cfg(unix)]
use std::os::unix::net::UnixDatagram;

/// 向 systemd 发送 READY/STOPPING 等状态；非 systemd 环境为 no-op。
pub fn notify(message: &str) -> std::io::Result<()> {
    let Some(socket) = std::env::var_os("NOTIFY_SOCKET") else {
        return Ok(());
    };
    #[cfg(unix)]
    {
        let socket = std::path::PathBuf::from(socket);
        let datagram = UnixDatagram::unbound()?;
        datagram.send_to(message.as_bytes(), socket)?;
    }
    #[cfg(not(unix))]
    let _ = (socket, message);
    Ok(())
}
