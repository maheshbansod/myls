use std::{
    collections::HashMap,
    io::{self, Read},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, instrument};

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
enum LSMessage {
    Request(LSMessageRequest),
    Notification,
    Response,
}

#[derive(Serialize, Deserialize, Debug)]
struct LSClientCapabilities {
    workspace: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "method", content = "params")]
#[serde(rename_all = "lowercase")]
enum LSMessageRequestBody {
    Initialize { capabilities: LSClientCapabilities },
    Unknown(serde_json::Value),
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
enum LSMessageResponseBody {
    Initialize(LSMessageResponseInitialize),
}

#[derive(Serialize, Deserialize, Debug)]
struct LSInfo {
    name: String,
    version: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct LSMessageResponseInitialize {
    /// todo: server capabilities
    capabilities: HashMap<String, String>,
    server_info: LSInfo,
}

impl LSMessageResponseInitialize {
    fn new(name: &str, version: &str) -> Self {
        Self {
            capabilities: HashMap::new(),
            server_info: LSInfo {
                name: name.to_owned(),
                version: version.to_owned(),
            },
        }
    }
}

type LSMessageRequest = JsonRpcRequest<LSMessageRequestBody>;

type LSMessageResponse = JsonRpcResponse<LSMessageResponseBody>;

impl LSMessageResponse {
    fn new(id: JsonRpcRequestId, body: LSMessageResponseBody) -> Self {
        Self {
            id,
            result: body,
            base: JsonRpcMessageBase {
                jsonrpc: "2.0".to_owned(),
            },
        }
    }
}

pub struct LServer {}
impl LServer {
    pub fn new() -> Self {
        Self {}
    }

    /// Blocks the thread and processes each message
    /// till the server exits
    pub fn run(self) {
        loop {
            match LServer::read() {
                Ok(message) => match message {
                    LSMessage::Request(request) => {
                        let request_body = request.request;
                        match self.message_response(request_body) {
                            Ok(response) => {
                                let id = request.id;
                                let response = LSMessageResponse::new(id, response);
                                self.respond(response);
                            }
                            Err(err) => {
                                debug!("Error: {err}")
                            }
                        }
                    }
                    _ => todo!(),
                },
                Err(err) => match err {
                    LSError::Parse(ParseError::JsonParsing(err)) => {
                        debug!("Couldn't parse JSON. {err}");
                    }
                    _ => debug!("Error occurred: {err:?}"),
                },
            }
            println!("{{}}");
        }
    }

    #[instrument]
    fn read() -> LSResult<LSMessage> {
        let mut buf = String::new();
        let mut content_length = None;
        loop {
            io::stdin()
                .read_line(&mut buf)
                .map_err(|err| ParseError::Io(err))?;
            debug!("buf: '{buf:?}'");
            if buf == "\r\n" {
                break;
            }
            let (name, value) = buf.split_once(":").ok_or(ParseError::Header)?;
            if name == "Content-Length" {
                content_length = Some(value.trim().parse().map_err(|_e| ParseError::Header)?);
            }
            if buf.ends_with("\r\n\r\n") {
                break;
            }
        }

        let content_length = content_length.ok_or(ParseError::Header)?;
        let header = LSHeader { content_length };
        let mut buf = vec![0u8; header.content_length as usize];
        io::stdin()
            .read_exact(&mut buf)
            .map_err(|err| ParseError::Io(err))?;
        let content = String::from_utf8_lossy(&buf);
        let content: LSMessage =
            serde_json::from_str(&content).map_err(|e| ParseError::JsonParsing(e))?;
        debug!("content: {:?}", content);

        Ok(content)
    }

    fn respond(&self, response: LSMessageResponse) {
        let response = serde_json::to_string(&response).unwrap();
        let content_length = response.len();
        let response = format!("Content-Length: {content_length}\r\n\r\n{response}");
        debug!(response);
        print!("{}", response)
    }

    fn message_response(&self, request: LSMessageRequestBody) -> LSResult<LSMessageResponseBody> {
        match request {
            LSMessageRequestBody::Initialize { capabilities: _ } => {
                Ok(LSMessageResponseBody::Initialize(
                    LSMessageResponseInitialize::new("myls", "0.0.1"),
                ))
            }
            LSMessageRequestBody::Unknown(value) => {
                debug!("Unknown request: {}", value.to_string());
                let request: JsonRpcRequest<JsonRpcGenericRequestBody> =
                    serde_json::from_value(value).map_err(|e| ParseError::JsonParsing(e))?;
                Err(LSError::MethodNotFound(request.request.method))
            }
        }
    }
}

struct LSHeader {
    content_length: u32,
}

type LSResult<T> = Result<T, LSError>;

#[derive(Error, Debug)]
enum LSError {
    #[error("Parsing error")]
    Parse(#[from] ParseError),
    #[error("Method unimplemented {0}")]
    MethodNotFound(String),
}

#[derive(Error, Debug)]
enum ParseError {
    #[error("IO error while parsing")]
    Io(#[from] io::Error),
    #[error("Header invalid")]
    Header,
    #[error("JSON parsing error. e: {0}")]
    JsonParsing(#[from] serde_json::Error),
}

#[derive(Serialize, Deserialize, Debug)]
struct JsonRpcMessageBase {
    /// 2.0
    jsonrpc: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct JsonRpcResponse<ResponseBody> {
    id: JsonRpcRequestId,
    result: ResponseBody,
    // method: String,
    // params: Params,
    #[serde(flatten)]
    base: JsonRpcMessageBase,
}

#[derive(Serialize, Deserialize, Debug)]
struct JsonRpcRequest<RequestBody> {
    id: JsonRpcRequestId,
    /// How do i enforce that ReqeustBody must have serde deserializer implemented
    /// such that it's a JSON object containing keys method: string and params: any
    #[serde(flatten)]
    request: RequestBody,
    // method: String,
    // params: Params,
    #[serde(flatten)]
    base: JsonRpcMessageBase,
}

#[derive(Serialize, Deserialize, Debug)]
struct JsonRpcGenericRequestBody {
    method: String,
    params: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
enum JsonRpcRequestId {
    String(String),
    Integer(i32),
}

// #[derive(Serialize, Deserialize)]
// #[serde(untagged)]
// enum JsonRpcMessage<Params=> {
//     Request(JsonRpcRequest<>),
//     Response(JsonRpcMessage)
// }
