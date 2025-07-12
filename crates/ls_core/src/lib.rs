use std::{
    collections::HashMap,
    fmt::Display,
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
    Initialize {
        capabilities: LSClientCapabilities,
    },
    #[serde(untagged)]
    Unknown {
        method: String,
        params: Option<serde_json::Value>,
    },
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

type LSMessageError = JsonRpcError<LSMessageErrorBody>;

impl LSMessageError {
    fn new(id: JsonRpcRequestId, body: LSMessageErrorBody) -> Self {
        Self {
            id,
            error: body,
            base: JsonRpcMessageBase {
                jsonrpc: "2.0".to_owned(),
            },
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct LSMessageErrorBody {
    code: i32,
    message: String,
    // data: ErrorData,
}

impl LSMessageErrorBody {
    fn from(kind: LSErrorKind) -> Self {
        LSMessageErrorBody {
            code: kind.code(),
            message: kind.message(),
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
                                self.respond_with_error(LSMessageError::new(
                                    request.id,
                                    LSMessageErrorBody::from(err.kind),
                                ));
                            }
                        }
                    }
                    _ => todo!(),
                },
                Err(err) => {
                    if let Some(request_id) = err.id {
                        self.respond_with_error(LSMessageError::new(
                            request_id,
                            LSMessageErrorBody::from(err.kind),
                        ));
                    } else {
                        debug!("Error: {err:?}");
                    }
                }
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
                .map_err(|err| LSError::parse(None, ParseError::Io(err)))?;
            debug!("buf: '{buf:?}'");
            if buf == "\r\n" {
                break;
            }
            let (name, value) = buf
                .split_once(":")
                .ok_or_else(|| LSError::parse(None, ParseError::Header))?;
            if name == "Content-Length" {
                content_length = Some(
                    value
                        .trim()
                        .parse()
                        .map_err(|_e| LSError::parse(None, ParseError::Header))?,
                );
            }
            if buf.ends_with("\r\n\r\n") {
                break;
            }
        }

        let content_length =
            content_length.ok_or_else(|| LSError::parse(None, ParseError::Header))?;
        let header = LSHeader { content_length };
        let mut buf = vec![0u8; header.content_length as usize];
        io::stdin()
            .read_exact(&mut buf)
            .map_err(|err| LSError::parse(None, ParseError::Io(err)))?;
        let content = String::from_utf8_lossy(&buf);
        // let content: LSMessage = serde_json::from_str(&content)
        //     .map_err(|e| LSError::parse(None, ParseError::JsonParsing((e, content.to_string()))))?;
        let content: LSMessage = match serde_json::from_str(&content) {
            Ok(content) => content,
            Err(e) => {
                debug!("content = {content:?}");
                let content_body: LSMessageRequest =
                    serde_json::from_str(&content).map_err(|e| {
                        LSError::parse(None, ParseError::JsonParsing((e, content.to_string())))
                    })?;
                debug!("b{:?}", content_body);
                return Err(LSError::parse(
                    None,
                    ParseError::JsonParsing((e, content.to_string())),
                ));
            }
        };
        debug!("content: {:?}", content);

        Ok(content)
    }

    fn respond_with_error(&self, response: LSMessageError) {
        let response = serde_json::to_string(&response).unwrap();
        let content_length = response.len();
        let response = format!("Content-Length: {content_length}\r\n\r\n{response}");
        debug!(response);
        print!("{}", response)
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
            LSMessageRequestBody::Unknown { method, params } => {
                debug!("Unknown request: {}. params={:?}", method, params);
                Err(LSError::method_not_found(None, method))
            }
        }
    }
}

struct LSHeader {
    content_length: u32,
}

type LSResult<T> = Result<T, LSError>;

#[derive(Debug)]
struct LSError {
    id: Option<JsonRpcRequestId>,
    kind: LSErrorKind,
}

impl LSError {
    fn method_not_found(id: Option<JsonRpcRequestId>, method: String) -> Self {
        Self {
            id,
            kind: LSErrorKind::MethodNotFound(method),
        }
    }
    fn parse(id: Option<JsonRpcRequestId>, e: ParseError) -> Self {
        Self {
            id,
            kind: LSErrorKind::Parse(e),
        }
    }
}

impl Display for LSError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LSError: [id={:?}] {:?}", self.id, self.kind)
    }
}

impl std::error::Error for LSError {}

#[derive(Error, Debug)]
enum LSErrorKind {
    #[error("Parse error")]
    Parse(ParseError),
    #[error("Method not found error")]
    MethodNotFound(String),
}

impl LSErrorKind {
    fn code(&self) -> i32 {
        match self {
            LSErrorKind::Parse(_) => -32700,
            LSErrorKind::MethodNotFound(_) => -32601,
        }
    }
    fn message(&self) -> String {
        match self {
            LSErrorKind::Parse(err) => err.to_string(),
            LSErrorKind::MethodNotFound(method) => format!("Method not found: {method}"),
        }
    }
}

#[derive(Error, Debug)]
enum ParseError {
    #[error("IO error while parsing")]
    Io(#[from] io::Error),
    #[error("Header invalid")]
    Header,
    #[error("JSON parsing error. e: {}", .0.0)]
    JsonParsing((serde_json::Error, String)),
}

#[derive(Serialize, Deserialize, Debug)]
struct JsonRpcMessageBase {
    /// 2.0
    jsonrpc: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct JsonRpcError<ErrorBody> {
    id: JsonRpcRequestId,
    error: ErrorBody,
    // method: String,
    // params: Params,
    #[serde(flatten)]
    base: JsonRpcMessageBase,
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
