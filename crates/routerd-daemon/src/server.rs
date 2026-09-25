use anyhow::Result;
use axum::Router;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use std::future::Future;
use std::path::Path;
use tokio::net::{TcpListener, UnixListener};
use tracing::{info, warn};

pub fn bind_standalone_unix(path: impl AsRef<Path>) -> Result<UnixListener> {
    let p = path.as_ref();
    if p.exists() {
        let _ = std::fs::remove_file(p);
    }
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(p)?;
    Ok(listener)
}

pub async fn bind_standalone_tcp(addr: &str) -> Result<TcpListener> {
    let listener = TcpListener::bind(addr).await?;
    Ok(listener)
}

pub async fn serve_tcp_gateway<F>(
    listener: TcpListener,
    app: Router,
    shutdown: F,
) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let local_addr = listener.local_addr().ok();
    info!("HTTP gateway listening on TCP {:?}", local_addr);

    let server = axum::serve(listener, app);
    server.with_graceful_shutdown(shutdown).await?;
    Ok(())
}

pub async fn serve_unix_gateway<F>(
    listener: UnixListener,
    app: Router,
    shutdown: F,
) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let local_addr = listener.local_addr().ok();
    info!("HTTP gateway listening on Unix socket {:?}", local_addr);

    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            res = listener.accept() => {
                match res {
                    Ok((stream, _)) => {
                        let app = app.clone();
                        tokio::spawn(async move {
                            let io = TokioIo::new(stream);
                            let service = TowerToHyperService::new(app);
                            let _ = Builder::new(TokioExecutor::new())
                                .serve_connection(io, service)
                                .await;
                        });
                    }
                    Err(e) => {
                        warn!("Unix gateway accept error: {}", e);
                    }
                }
            }
        }
    }
    Ok(())
}
