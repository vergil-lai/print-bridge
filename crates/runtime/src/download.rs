use crate::{
    config::LimitsConfig,
    protocol::{is_pdf_data_url, validate_file_url},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures_util::StreamExt;
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

/// 下载浏览器提供的打印文件时产生的错误。
#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("invalid file url")]
    InvalidFileUrl,
    #[error("file too large")]
    FileTooLarge,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Request(reqwest::Error),
}

/// 文件下载和临时文件写入的统一返回结果。
pub type DownloadResult<T> = Result<T, DownloadError>;

/// 把已校验的 HTTP(S) 或 PDF data URL 文件保存到唯一临时路径。
pub async fn download_to_temp(file_url: &str, limits: &LimitsConfig) -> DownloadResult<PathBuf> {
    download_to_path(file_url, limits, temp_download_path()).await
}

/// 流式写入文件到指定路径，同时执行配置的大小和超时限制。
async fn download_to_path(
    file_url: &str,
    limits: &LimitsConfig,
    path: PathBuf,
) -> DownloadResult<PathBuf> {
    if is_pdf_data_url(file_url) {
        return write_pdf_data_url_to_path(file_url, limits, path).await;
    }

    let url = validate_file_url(file_url).map_err(|_| DownloadError::InvalidFileUrl)?;
    let max_bytes = limits.max_file_size_mb.saturating_mul(1024 * 1024);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(limits.download_timeout_seconds))
        .build()
        .map_err(DownloadError::Request)?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(DownloadError::Request)?
        .error_for_status()
        .map_err(DownloadError::Request)?;
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(DownloadError::FileTooLarge);
    }

    let mut file = tokio::fs::File::create(&path).await?;
    let mut downloaded = 0_u64;
    let mut stream = response.bytes_stream();

    // Content-Length 可能不存在或不准确，所以还要在流式下载时
    // 再次执行字节限制，并在失败时删除残留文件。
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|error| cleanup_download(&path, DownloadError::Request(error)))?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > max_bytes {
            return Err(cleanup_download(&path, DownloadError::FileTooLarge));
        }
        file.write_all(&chunk)
            .await
            .map_err(|error| cleanup_download(&path, DownloadError::Io(error)))?;
    }

    file.flush()
        .await
        .map_err(|error| cleanup_download(&path, DownloadError::Io(error)))?;
    Ok(path)
}

/// 解码浏览器 SDK 生成的 base64 PDF data URL，并写入临时文件。
async fn write_pdf_data_url_to_path(
    file_url: &str,
    limits: &LimitsConfig,
    path: PathBuf,
) -> DownloadResult<PathBuf> {
    validate_file_url(file_url).map_err(|_| DownloadError::InvalidFileUrl)?;
    let max_bytes = limits.max_file_size_mb.saturating_mul(1024 * 1024);
    let (_, encoded) = file_url
        .split_once(',')
        .ok_or(DownloadError::InvalidFileUrl)?;
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| DownloadError::InvalidFileUrl)?;

    if bytes.len() as u64 > max_bytes {
        return Err(DownloadError::FileTooLarge);
    }

    tokio::fs::write(&path, bytes)
        .await
        .map_err(|error| cleanup_download(&path, DownloadError::Io(error)))?;
    Ok(path)
}

/// 删除部分下载文件，并返回原始错误继续向上传递。
fn cleanup_download(path: &Path, error: DownloadError) -> DownloadError {
    let _ = std::fs::remove_file(path);
    error
}

/// 为传入打印文档生成低冲突风险的临时文件路径。
fn temp_download_path() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();

    std::env::temp_dir().join(format!(
        "print-bridge-download-{timestamp}-{}.tmp",
        Uuid::new_v4()
    ))
}

#[cfg(test)]
mod tests {
    use super::{download_to_path, download_to_temp, DownloadError};
    use crate::config::LimitsConfig;
    use std::{
        io::{Read, Write},
        net::{SocketAddr, TcpListener},
        path::PathBuf,
        thread,
    };

    #[tokio::test]
    async fn download_to_temp_saves_successful_response() {
        let server = TestServer::start(|stream| {
            write_response(stream, "200 OK", Some(5), b"hello");
        });

        let path = download_to_temp(&server.url(), &limits(1)).await.unwrap();

        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"hello");
        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn download_to_temp_decodes_pdf_data_url() {
        let path = download_to_temp(
            "data:application/pdf;base64,JVBERi0xLjcKJSVFT0Y=",
            &limits(1),
        )
        .await
        .unwrap();

        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"%PDF-1.7\n%%EOF");
        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn download_to_temp_rejects_non_pdf_data_url() {
        let result =
            download_to_temp("data:text/html;base64,PGgxPkxhYmVsPC9oMT4=", &limits(1)).await;

        assert!(matches!(result, Err(DownloadError::InvalidFileUrl)));
    }

    #[tokio::test]
    async fn download_to_temp_rejects_data_url_over_limit() {
        let path = test_download_path("data-url-limit");
        let _ = std::fs::remove_file(&path);

        let result = download_to_path(
            "data:application/pdf;base64,JVBERi0xLjcKJSVFT0Y=",
            &limits(0),
            path.clone(),
        )
        .await;

        assert!(matches!(result, Err(DownloadError::FileTooLarge)));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn download_to_temp_returns_request_error_for_http_error_status() {
        let server = TestServer::start(|stream| {
            write_response(stream, "404 Not Found", Some(9), b"not found");
        });

        let result = download_to_temp(&server.url(), &limits(1)).await;

        assert!(matches!(result, Err(DownloadError::Request(_))));
    }

    #[tokio::test]
    async fn download_to_temp_rejects_content_length_over_limit_before_creating_file() {
        let path = test_download_path("content-length-limit");
        let _ = std::fs::remove_file(&path);
        let server = TestServer::start(|stream| {
            write_response(stream, "200 OK", Some(1), b"x");
        });

        let result = download_to_path(&server.url(), &limits(0), path.clone()).await;

        assert!(matches!(result, Err(DownloadError::FileTooLarge)));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn download_to_temp_removes_partial_file_when_stream_exceeds_limit() {
        let path = test_download_path("stream-limit");
        let _ = std::fs::remove_file(&path);
        let server = TestServer::start(|stream| {
            write_response(stream, "200 OK", None, b"x");
        });

        let result = download_to_path(&server.url(), &limits(0), path.clone()).await;

        assert!(matches!(result, Err(DownloadError::FileTooLarge)));
        assert!(!path.exists());
    }

    struct TestServer {
        address: SocketAddr,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl TestServer {
        fn start(handler: fn(std::net::TcpStream)) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let handle = thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                handler(stream);
            });

            Self {
                address,
                handle: Some(handle),
            }
        }

        fn url(&self) -> String {
            format!("http://{}", self.address)
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take() {
                handle.join().unwrap();
            }
        }
    }

    fn write_response(
        mut stream: std::net::TcpStream,
        status: &str,
        content_length: Option<usize>,
        body: &[u8],
    ) {
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        let header = match content_length {
            Some(length) => format!(
                "HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
            ),
            None => format!("HTTP/1.1 {status}\r\nConnection: close\r\n\r\n"),
        };
        stream.write_all(header.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
    }

    fn limits(max_file_size_mb: u64) -> LimitsConfig {
        LimitsConfig {
            max_file_size_mb,
            max_batch_jobs: 20,
            max_copies: 100,
            download_timeout_seconds: 5,
        }
    }

    fn test_download_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "print-bridge-download-test-{}-{name}.tmp",
            std::process::id()
        ))
    }
}
