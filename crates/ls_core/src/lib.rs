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
    Notification(LSMessageNotification),
    Response,
}

#[derive(Serialize, Deserialize, Debug)]
struct LSClientCapabilities {
    workspace: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "method", content = "params")]
#[serde(rename_all = "lowercase")]
enum LSMessageNotificationBody {
    Initialized {},
    Exit,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "method", content = "params")]
#[serde(rename_all = "lowercase")]
enum LSMessageRequestBody {
    Initialize {
        capabilities: LSClientCapabilities,
    },
    Shutdown,
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
    Shutdown,
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
type LSMessageNotification = JsonRpcNotification<LSMessageNotificationBody>;

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
    fn from(kind: LSError) -> Self {
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
        // kinda a fail safe thing - avoids clogging logs
        let mut error_count = 0;
        loop {
            match LServer::read() {
                Ok(message) => {
                    error_count = 0;
                    match message {
                        LSMessage::Request(request) => {
                            let request_body = request.request;
                            match self.message_response(request_body) {
                                Ok(response) => {
                                    let id = request.id;
                                    let response = LSMessageResponse::new(id, response);
                                    self.respond(&response);
                                }
                                Err(err) => {
                                    self.respond_with_error(LSMessageError::new(
                                        request.id,
                                        LSMessageErrorBody::from(err),
                                    ));
                                }
                            }
                        }
                        LSMessage::Notification(notification) => match notification.notification {
                            LSMessageNotificationBody::Initialized {} => {
                                debug!("initialized!");
                            }
                            LSMessageNotificationBody::Exit => {
                                break;
                            }
                        },
                        _ => todo!(),
                    }
                }
                Err(err) => {
                    error_count += 1;
                    debug!("Error: {err:?}");
                    if error_count == 10 {
                        break;
                    }
                }
            }
        }

        debug!("exiting");
    }

    #[instrument]
    fn read() -> Result<LSMessage, ParseError> {
        let mut buf = String::new();
        let mut content_length = None;
        loop {
            debug!("Waiting for input");
            io::stdin()
                .read_line(&mut buf)
                .map_err(|err| ParseError::Io(err))?;

            debug!("buf: '{buf:?}'");
            if buf.len() == 0 {
                break;
            }
            if buf == "\r\n" {
                break;
            }
            let (name, value) = buf.split_once(":").ok_or_else(|| ParseError::Header)?;
            if name == "Content-Length" {
                content_length = Some(value.trim().parse().map_err(|_e| ParseError::Header)?);
            }
            if buf.ends_with("\r\n\r\n") {
                break;
            }
        }

        let content_length = content_length.ok_or_else(|| ParseError::Header)?;
        let header = LSHeader { content_length };
        let mut buf = vec![0u8; header.content_length as usize];
        io::stdin()
            .read_exact(&mut buf)
            .map_err(|err| ParseError::Io(err))?;
        let content = String::from_utf8_lossy(&buf);
        let content: LSMessage = serde_json::from_str(&content)
            .map_err(|e| ParseError::JsonParsing((e, content.to_string())))?;
        debug!("content: {:?}", content);

        Ok(content)
    }

    fn respond_with_error(&self, response: LSMessageError) {
        let response = serde_json::to_string(&response).unwrap();
        let content_length = response.len();
        let response = format!("Content-Length: {content_length}\r\n\r\n{response}");
        debug!(response);
        println!("{}", response)
    }

    fn respond(&self, response: &LSMessageResponse) {
        let response = serde_json::to_string(&response).unwrap();
        let content_length = response.len();
        let response = format!("Content-Length: {content_length}\r\n\r\n{response}");
        debug!(response);
        println!("{}", response)
    }

    fn message_response(&self, request: LSMessageRequestBody) -> LSResult<LSMessageResponseBody> {
        match request {
            LSMessageRequestBody::Initialize { capabilities: _ } => {
                Ok(LSMessageResponseBody::Initialize(
                    LSMessageResponseInitialize::new("myls", "0.0.1"),
                ))
            }
            LSMessageRequestBody::Shutdown => Ok(LSMessageResponseBody::Shutdown),
            LSMessageRequestBody::Unknown { method, params } => {
                debug!("Unknown request: {}. params={:?}", method, params);
                Err(LSError::MethodNotFound(method))
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
    // #[error("Parse error")]
    // Parse(ParseError),
    #[error("Method not found error")]
    MethodNotFound(String),
}

impl LSError {
    fn code(&self) -> i32 {
        match self {
            // LSErrorKind::Parse(_) => -32700,
            LSError::MethodNotFound(_) => -32601,
        }
    }
    fn message(&self) -> String {
        match self {
            // LSErrorKind::Parse(err) => err.to_string(),
            LSError::MethodNotFound(method) => format!("Method not found: {method}"),
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
    #[serde(flatten)]
    base: JsonRpcMessageBase,
}

#[derive(Serialize, Deserialize, Debug)]
struct JsonRpcResponse<ResponseBody> {
    id: JsonRpcRequestId,
    result: ResponseBody,
    #[serde(flatten)]
    base: JsonRpcMessageBase,
}

#[derive(Serialize, Deserialize, Debug)]
struct JsonRpcNotification<NotificationBody> {
    /// How do i enforce that NotificationBody must have serde deserializer implemented
    /// such that it's a JSON object containing keys method: string and params: any
    #[serde(flatten)]
    notification: NotificationBody,
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
