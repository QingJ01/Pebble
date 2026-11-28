use std::collections::HashMap;
use std::sync::Mutex;

use super::*;
use crate::protocol::{BridgeResponse, ErrorCode};
use crate::transport::{exchange, ConnectFuture};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Approve,
    Deny,
    Offline,
    Revoked,
}

/// A stand-in for the Pebble app.
struct FakeApp {
    mode: Mutex<Mode>,
    seen: Mutex<Vec<BridgeRequest>>,
}

impl Connector for FakeApp {
    fn send<'a>(&'a self, request: &'a BridgeRequest, _wait: Duration) -> ConnectFuture<'a> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(request.clone());
            let mode = *self.mode.lock().unwrap();
            match (mode, request) {
                (Mode::Offline, _) => Err("refused".into()),
                (Mode::Deny, BridgeRequest::Pair { .. }) => {
                    Ok(BridgeResponse::error(ErrorCode::Denied, "no"))
                }
                (_, BridgeRequest::Pair { client }) => {
                    Ok(BridgeResponse::token(format!("tok-{client}")))
                }
                (Mode::Revoked, BridgeRequest::Call { .. }) => {
                    Ok(BridgeResponse::error(ErrorCode::Unauthorized, "revoked"))
                }
                (_, BridgeRequest::Call { token, tool, .. }) => Ok(BridgeResponse::ok(
                    json!({"tool": tool, "token_seen": token}),
                )),
            }
        })
    }
}

#[derive(Default)]
struct MemoryTokens(Mutex<HashMap<String, String>>);

impl TokenStore for MemoryTokens {
    fn get(&self, client: &str) -> Option<String> {
        self.0.lock().unwrap().get(client).cloned()
    }
    fn set(&self, client: &str, token: &str) {
        self.0.lock().unwrap().insert(client.into(), token.into());
    }
    fn delete(&self, client: &str) {
        self.0.lock().unwrap().remove(client);
    }
}

fn bridge(mode: Mode) -> Arc<Bridge<FakeApp, MemoryTokens>> {
    Bridge::new(
        FakeApp {
            mode: Mutex::new(mode),
            seen: Mutex::new(Vec::new()),
        },
        MemoryTokens::default(),
    )
}

fn init(version: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": version, "clientInfo": {"name": "Claude Desktop"}, "capabilities": {}}})
}

fn call(tool: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": tool, "arguments": {"query": "x"}}})
}

fn text(answer: &Value) -> &str {
    answer["result"]["content"][0]["text"].as_str().unwrap()
}

#[tokio::test]
async fn pairs_on_initialize_then_forwards_calls_with_the_token() {
    let b = bridge(Mode::Approve);
    let answer = handle(&b, init("2025-03-26")).await.unwrap();
    assert_eq!(answer["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(answer["result"]["serverInfo"]["name"], "pebble");

    let tools = handle(
        &b,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await
    .unwrap();
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 9);
    assert!(names.contains(&"mail_list_accounts") && names.contains(&"mail_proposal_status"));
    assert!(
        !names
            .iter()
            .any(|n| n.contains("send") || n.contains("delete")),
        "no tool sends or deletes"
    );

    let result = handle(&b, call("mail_search")).await.unwrap();
    assert_eq!(result["result"]["isError"], false);
    assert_eq!(
        result["result"]["structuredContent"]["tool"], "mail_search",
        "structured content too"
    );
    assert!(
        text(&result).contains("tok-Claude Desktop"),
        "{}",
        text(&result)
    );
    let pairs = b
        .connector
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|r| matches!(r, BridgeRequest::Pair { .. }))
        .count();
    assert_eq!(pairs, 1, "one pairing only");
}

#[tokio::test]
async fn unknown_versions_methods_and_notifications() {
    let b = bridge(Mode::Approve);
    assert_eq!(
        handle(&b, init("1999-01-01")).await.unwrap()["result"]["protocolVersion"],
        "2025-06-18"
    );
    assert!(handle(
        &b,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
    )
    .await
    .is_none());
    assert_eq!(
        handle(
            &b,
            json!({"jsonrpc": "2.0", "id": 3, "method": "sampling/x"})
        )
        .await
        .unwrap()["error"]["code"],
        -32601
    );
    assert_eq!(
        handle(&b, json!({"jsonrpc": "2.0", "id": 4, "method": "ping"}))
            .await
            .unwrap()["result"],
        json!({})
    );
}

#[tokio::test]
async fn explains_offline_denied_and_revoked() {
    let offline = bridge(Mode::Offline);
    handle(&offline, init("2025-06-18")).await;
    let r = handle(&offline, call("mail_search")).await.unwrap();
    assert_eq!(r["result"]["isError"], true);
    assert!(text(&r).contains("Open Pebble"));

    let denied = bridge(Mode::Deny);
    handle(&denied, init("2025-06-18")).await;
    let r = handle(&denied, call("mail_search")).await.unwrap();
    assert!(text(&r).contains("declined"));

    let revoked = bridge(Mode::Revoked);
    revoked.tokens.set("Claude Desktop", "old");
    handle(&revoked, init("2025-06-18")).await;
    let r = handle(&revoked, call("mail_search")).await.unwrap();
    assert!(text(&r).contains("revoked"));
    tokio::task::yield_now().await;
    assert_ne!(
        revoked.tokens.get("Claude Desktop").as_deref(),
        Some("old"),
        "the stale token is dropped"
    );
}

#[tokio::test]
async fn serves_lines_over_stdio() {
    let b = bridge(Mode::Approve);
    let input = format!(
        "{}\n{}\n",
        init("2025-06-18"),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
    );
    let (client, server) = tokio::io::duplex(64 * 1024);
    run_stdio(b, input.as_bytes(), server).await.unwrap();
    let mut lines = BufReader::new(client).lines();
    let mut ids = Vec::new();
    while let Some(line) = lines.next_line().await.unwrap() {
        ids.push(
            serde_json::from_str::<Value>(&line).unwrap()["id"]
                .as_i64()
                .unwrap(),
        );
    }
    ids.sort();
    assert_eq!(ids, vec![1, 2]);
}

#[tokio::test]
async fn exchange_reads_one_answer_line() {
    let (mut app, bridge_side) = tokio::io::duplex(4096);
    let server = tokio::spawn(async move {
        let (read, mut write) = tokio::io::split(&mut app);
        let mut line = String::new();
        BufReader::new(read).read_line(&mut line).await.unwrap();
        assert!(line.contains("\"op\":\"pair\""));
        write
            .write_all(b"{\"ok\":true,\"token\":\"t1\"}\n")
            .await
            .unwrap();
    });
    let answer = exchange(
        bridge_side,
        &BridgeRequest::Pair { client: "x".into() },
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(answer.token.as_deref(), Some("t1"));
    server.await.unwrap();
}

/// The real socket transport, against a small stand-in for Pebble.
#[cfg(unix)]
#[tokio::test]
async fn pairs_and_calls_over_a_real_socket() {
    use crate::transport::SocketConnector;
    let dir = std::env::temp_dir().join(format!("pebble-mcp-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.sock");
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let (read, mut write) = tokio::io::split(stream);
                let mut line = String::new();
                BufReader::new(read).read_line(&mut line).await.unwrap();
                let answer = match serde_json::from_str::<BridgeRequest>(line.trim()).unwrap() {
                    BridgeRequest::Pair { .. } => BridgeResponse::token("sock-token".into()),
                    BridgeRequest::Call { token, .. } if token == "sock-token" => {
                        BridgeResponse::ok(json!({"results": [1, 2]}))
                    }
                    BridgeRequest::Call { .. } => {
                        BridgeResponse::error(ErrorCode::Unauthorized, "no")
                    }
                };
                let mut text = serde_json::to_string(&answer).unwrap();
                text.push('\n');
                write.write_all(text.as_bytes()).await.unwrap();
            });
        }
    });
    let b = Bridge::new(
        SocketConnector::new(path.to_string_lossy().into_owned()),
        MemoryTokens::default(),
    );
    handle(&b, init("2025-06-18")).await;
    let r = handle(&b, call("mail_search")).await.unwrap();
    assert_eq!(r["result"]["isError"], false, "{}", text(&r));
    assert!(text(&r).contains("results"));

    let gone = Bridge::new(
        SocketConnector::new(dir.join("none.sock").to_string_lossy().into_owned()),
        MemoryTokens::default(),
    );
    let r = handle(&gone, call("mail_search")).await.unwrap();
    assert!(text(&r).contains("Open Pebble"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A call the client cancels gets no answer; others still do.
#[tokio::test]
async fn cancelled_calls_get_no_answer() {
    struct Slow;
    impl Connector for Slow {
        fn send<'a>(&'a self, request: &'a BridgeRequest, _wait: Duration) -> ConnectFuture<'a> {
            Box::pin(async move {
                if let BridgeRequest::Call { .. } = request {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                Ok(BridgeResponse::token("t".into()))
            })
        }
    }
    let tokens = MemoryTokens::default();
    tokens.set("MCP client", "t");
    let b = Bridge::new(Slow, tokens);
    let input = format!(
        "{}\n{}\n{}\n",
        call("mail_search"),
        json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 7}}),
        json!({"jsonrpc": "2.0", "id": 8, "method": "ping"})
    );
    let (client, server) = tokio::io::duplex(64 * 1024);
    run_stdio(b, input.as_bytes(), server).await.unwrap();
    let mut lines = BufReader::new(client).lines();
    let mut ids = Vec::new();
    while let Some(line) = lines.next_line().await.unwrap() {
        ids.push(serde_json::from_str::<Value>(&line).unwrap()["id"].clone());
    }
    assert_eq!(ids, vec![json!(8)], "only the ping is answered");
}
