use std::io::{self, ErrorKind};
use std::num::NonZeroUsize;
use std::time::Duration;

use gt_session_bridge::{Recording, Recordings, Reply, Request};
use rmcp::ServiceExt as _;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf,
};

use super::{Bridge, COMPANION_NAME, Companion, DEFAULT_PAGE_LIMIT, START_DESKTOP, VERSION};

#[tokio::test]
async fn the_companion_lists_one_tool_and_returns_a_recording_page() {
    let (server, stream) = tokio::io::duplex(8192);
    let worker = tokio::spawn(async move {
        Companion {
            bridge: Bridge::Fixture(self::recording_page),
        }
        .serve(server)
        .await
        .unwrap()
        .waiting()
        .await
        .unwrap();
    });
    let mut client = Client::new(stream);
    let initialized = client
        .exchange(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "1"}
            }
        }))
        .await;
    assert_eq!(
        initialized.pointer("/result/protocolVersion"),
        Some(&json!("2025-11-25"))
    );
    assert_eq!(
        initialized.pointer("/result/serverInfo/name"),
        Some(&json!(COMPANION_NAME))
    );
    assert_eq!(
        initialized.pointer("/result/serverInfo/version"),
        Some(&json!(VERSION))
    );
    client
        .send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await;

    let listed = client
        .exchange(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
        .await;
    let tools = listed
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .unwrap();
    let [tool] = tools.as_slice() else {
        panic!("expected one tool");
    };
    assert_eq!(tool.get("name"), Some(&json!("list_recordings")));
    assert_eq!(
        tool.pointer("/annotations/readOnlyHint"),
        Some(&json!(true))
    );
    assert!(tool.pointer("/inputSchema/properties/after").is_some());
    assert!(tool.pointer("/inputSchema/properties/limit").is_some());

    let absent = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "list_recordings"}
        }))
        .await;
    assert_eq!(absent.pointer("/result/isError"), Some(&json!(true)));
    assert_eq!(
        absent.pointer("/result/content/0/text"),
        Some(&json!(START_DESKTOP))
    );

    let page = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": {"name": "list_recordings", "arguments": {"after": 6, "limit": 1}}
        }))
        .await;
    assert_eq!(page.pointer("/result/isError"), Some(&json!(false)));
    let expected = json!({
        "recordings": [{
            "id": 7, "display_name": "Survey", "filename": "survey.gtd",
            "title": null, "device": null, "time_range": null, "track_count": 2
        }],
        "next_after": 7
    });
    assert_eq!(page.pointer("/result/structuredContent"), Some(&expected));
    assert_eq!(
        page.pointer("/result/content/0/text"),
        Some(&json!(expected.to_string()))
    );
    let invalid = client
        .exchange(json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": {"name": "list_recordings", "arguments": {"after": 6, "limit": 0}}
        }))
        .await;
    assert_eq!(invalid.pointer("/result/isError"), Some(&json!(true)));
    assert!(
        invalid
            .pointer("/result/content/0/text")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("failed to deserialize parameters"))
    );
    drop(client);
    tokio::time::timeout(Duration::from_secs(2), worker)
        .await
        .unwrap()
        .unwrap();
}

fn recording_page(Request::ListRecordings { after, limit }: Request) -> io::Result<Reply> {
    if after.is_none() && limit == DEFAULT_PAGE_LIMIT {
        return Err(ErrorKind::NotFound.into());
    }
    if after != Some(6) || limit != NonZeroUsize::MIN {
        return Err(ErrorKind::InvalidInput.into());
    }
    Ok(Reply::Recordings {
        page: Recordings {
            recordings: vec![Recording {
                id: 7,
                display_name: "Survey".to_owned(),
                filename: "survey.gtd".to_owned(),
                title: None,
                device: None,
                time_range: None,
                track_count: 2,
            }],
            next_after: Some(7),
        },
    })
}

struct Client {
    input: Lines<BufReader<ReadHalf<DuplexStream>>>,
    output: WriteHalf<DuplexStream>,
}

impl Client {
    fn new(stream: DuplexStream) -> Self {
        let (input, output) = tokio::io::split(stream);
        Self {
            input: BufReader::new(input).lines(),
            output,
        }
    }

    async fn exchange(&mut self, message: Value) -> Value {
        self.send(message).await;
        let line = tokio::time::timeout(Duration::from_secs(2), self.input.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }

    async fn send(&mut self, message: Value) {
        let line = format!("{message}\n");
        self.output.write_all(line.as_bytes()).await.unwrap();
    }
}
