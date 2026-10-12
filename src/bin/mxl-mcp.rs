// Copyright 2026 Mixlayer Inc. Licensed under Apache-2.0. See LICENSE.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let service = match rmcp::serve_server(mixlayer::mcp::MixlayerServer::new(), rmcp::transport::io::stdio()).await {
        Ok(s) => s,
        // The client went away before initializing (closed stdin): a normal
        // shutdown, as in the Go server.
        Err(rmcp::service::ServerInitializeError::ConnectionClosed(_)) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    match service.waiting().await {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}
