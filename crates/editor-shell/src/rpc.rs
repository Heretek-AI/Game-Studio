use std::io::{BufRead, Write};

/// Loopback JSON-RPC bus (spec 04 §1–§2).
///
/// Transport note (Rust 1.82 MSRV): `jsonrpsee` pulls `hashbrown 0.17`
/// (edition2024, vetted failure in a scratch crate) and is deferred to the
/// toolchain-bump ADR. Both transports below speak the identical method table
/// (`mcp_gate::handle_request_line`) with identical envelopes and version
/// rejection, which is what `rpc-conformance --transport ws,stdio` asserts:
/// - `stdio`: newline-delimited JSON-RPC on stdin/stdout (agents/CI).
/// - `ws`: the same framing over loopback TCP `127.0.0.1:39002` (editor).
///   Full RFC6455 upgrade framing is the deferred delta; the RPC surface is
///   frozen either way.
pub const WS_PORT: u16 = 39002;

pub fn serve_stdio() {
    mcp_gate::serve_stdio();
}

pub fn serve_tcp(listener: std::net::TcpListener) {
    mcp_gate::serve_tcp_loopback(listener);
}

pub fn bind_loopback(port: u16) -> anyhow::Result<std::net::TcpListener> {
    Ok(std::net::TcpListener::bind(format!("127.0.0.1:{port}"))?)
}

/// WS-transport round trip used by conformance: bind an ephemeral loopback
/// port, serve one handler thread, send `requests` line-by-line, collect
/// one response line per request.
pub fn ws_round_trip(requests: &[String]) -> anyhow::Result<Vec<String>> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let count = requests.len();
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            let mut reader =
                std::io::BufReader::new(stream.try_clone().expect("tcp clone succeeds"));
            let mut writer = stream;
            let mut line = String::new();
            for _ in 0..count {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let response = mcp_gate::handle_request_line(line.trim());
                        let _ = writeln!(writer, "{response}");
                        let _ = writer.flush();
                    }
                    Err(_) => break,
                }
            }
        }
    });
    let mut stream = std::net::TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    let mut out = Vec::with_capacity(requests.len());
    for request in requests {
        writeln!(stream, "{request}")?;
    }
    let mut reader = std::io::BufReader::new(stream);
    let mut line = String::new();
    for _ in 0..requests.len() {
        line.clear();
        reader.read_line(&mut line)?;
        out.push(line.trim().to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_transport_echoes_method_table() {
        let requests = vec![
            r#"{"jsonrpc":"2.0","id":1,"method":"scene.query/v1","params":{}}"#.to_string(),
            r#"{"jsonrpc":"2.0","id":2,"method":"scene.query/v9"}"#.to_string(),
        ];
        let responses = ws_round_trip(&requests).unwrap();
        assert_eq!(responses.len(), 2);
        let ok: serde_json::Value = serde_json::from_str(&responses[0]).unwrap();
        assert!(ok.get("result").is_some());
        let version: serde_json::Value = serde_json::from_str(&responses[1]).unwrap();
        assert_eq!(version["error"]["code"], serde_json::json!(-32001));
    }
}
