//! `bridged --bridge-mcp-visualize` is the stdio MCP server chat harnesses
//! spawn. It must answer over stdio and exit cleanly on EOF, without touching
//! a data directory.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

#[test]
fn bridged_helper_flag_serves_stdio() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bridged"))
        .arg("--bridge-mcp-visualize")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("bridged spawns");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut ask = |message: Value| -> Value {
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).expect("one JSON response per line")
    };

    let init = ask(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}}));
    assert_eq!(init["result"]["serverInfo"]["name"], "bridge");
    let tools = ask(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    assert_eq!(tools["result"]["tools"][0]["name"], "visualize");
    let call = ask(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "visualize", "arguments": {
        "version": 1, "title": "Spend",
        "sources": [{"id": "s", "kind": "user", "ref": "the user's numbers"}],
        "blocks": [{"family": "chart", "form": "bar", "sourceIds": ["s"], "vegaLite": {
            "mark": "bar",
            "data": {"values": [{"k": "a", "v": 1}, {"k": "b", "v": 2}, {"k": "c", "v": 3}]},
            "encoding": {"x": {"field": "k"}, "y": {"field": "v"}}
        }}]
    }}}));
    assert_eq!(call["result"]["isError"], false, "{call}");

    drop(stdin);
    let status = child.wait().expect("bridged exits");
    assert!(status.success(), "{status:?}");
}
