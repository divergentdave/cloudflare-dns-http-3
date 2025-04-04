use std::{
    future::poll_fn,
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};

use bytes::Buf;
use http::{Method, Request};
use quinn::{crypto::rustls::QuicClientConfig, Endpoint, TransportConfig};
use rustls::KeyLogFile;
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::sync::oneshot;

static QUERY_MESSAGE: &[u8] = include_bytes!("../request.bin");

#[tokio::main]
async fn main() {
    // Set up logging.
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    // Set up rustls provider.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .unwrap();

    run(true, false).await;
    run(true, true).await;
    run(false, false).await;
    run(false, true).await;
}

async fn run(send_grease: bool, send_content_length: bool) {
    println!("GREASE: {send_grease}, Content-Length: {send_content_length}");

    // Configure rustls.
    let mut rustls_config = rustls::ClientConfig::builder()
        .with_platform_verifier()
        .with_no_client_auth();
    rustls_config.alpn_protocols = vec![b"h3".to_vec()];
    rustls_config.key_log = Arc::new(KeyLogFile::new());

    // Configure quinn.
    let transport_config = Arc::new(TransportConfig::default());
    let mut quinn_config =
        quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(rustls_config).unwrap()));
    quinn_config.transport_config(transport_config);

    // Initiate the QUIC connection, then initiate the HTTP/3 connection.
    let endpoint = Endpoint::client((Ipv4Addr::UNSPECIFIED, 0).into()).unwrap();
    let connection = endpoint
        .connect_with(
            quinn_config,
            SocketAddr::new(Ipv4Addr::new(1, 1, 1, 1).into(), 443),
            "cloudflare-dns.com",
        )
        .unwrap()
        .await
        .unwrap();
    let (mut h3_connection, mut sender) = h3::client::builder()
        .send_grease(send_grease)
        .build::<_, _, &[u8]>(h3_quinn::Connection::new(connection))
        .await
        .unwrap();

    // Run connection-related processing on a background task.
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    let connection_handle = tokio::spawn(async move {
        tokio::select! {
            closed = poll_fn(|cx| h3_connection.poll_close(cx)) => closed?,
            _ = &mut shutdown_rx => h3_connection.shutdown(0).await?,
        }
        Ok::<(), h3::Error>(())
    });

    // Send the request.
    let mut request_builder = Request::builder()
        .uri("https://cloudflare-dns.com/dns-query")
        .method(Method::POST)
        .header("Content-Type", "application/dns-message")
        .header("Accept", "application/dns-message");
    if send_content_length {
        request_builder =
            request_builder.header("Content-Length", format!("{}", QUERY_MESSAGE.len()))
    }
    let request = request_builder.body(()).unwrap();
    let mut request_stream = sender.send_request(request).await.unwrap();
    request_stream.send_data(QUERY_MESSAGE).await.unwrap();
    request_stream.finish().await.unwrap();

    // Receive the response.
    let response = request_stream.recv_response().await.unwrap();
    println!("Status: {}", response.status());
    println!("Headers: {:#?}", response.headers());
    while let Some(mut body_buf) = request_stream.recv_data().await.unwrap() {
        let mut body_vec = vec![0u8; body_buf.remaining()];
        body_buf.copy_to_slice(&mut body_vec);
        let body_str = String::from_utf8_lossy(&body_vec);
        println!("Body: {body_str:?} {body_vec:02x?}");
    }

    // Clean up the connection.
    shutdown_tx.send(()).unwrap();
    connection_handle.await.unwrap().unwrap();

    println!();
}
