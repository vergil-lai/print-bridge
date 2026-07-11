/// 等待 SIGINT 或 SIGTERM，触发一次优雅关闭。
pub async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result?,
            _ = terminate.recv() => {}
        }
        Ok(())
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}
