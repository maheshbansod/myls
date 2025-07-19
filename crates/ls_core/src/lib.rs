use std::{
    fs,
    io::{self, Read},
};

use serde::{Deserialize, Serialize};
// use streaming_iterator::StreamingIterator;
use thiserror::Error;
use tracing::{debug, instrument};
use tree_sitter::{Query, QueryCursor, StreamingIterator};

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
enum LSMessage {
    Request(LSMessageRequest),
    Notification(LSMessageNotification),
    Response,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct DefinitionClientCapabilities {
    link_support: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct TextDocumentClientCapabilities {
    definition: DefinitionClientCapabilities,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct LSClientCapabilities {
    workspace: serde_json::Value,
    text_document: Option<TextDocumentClientCapabilities>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "method", content = "params")]
#[serde(rename_all = "lowercase")]
enum LSMessageNotificationBody {
    Initialized {},
    Exit,
}

#[derive(Serialize, Deserialize, Debug)]
struct LsTypePosition {
    character: u32,
    line: u32,
}

#[derive(Serialize, Deserialize, Debug)]
struct LsTypeTextDocument {
    uri: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "method", content = "params")]
#[serde(rename_all = "lowercase")]
enum LSMessageRequestBody {
    Initialize {
        capabilities: LSClientCapabilities,
    },
    Shutdown,
    #[serde(rename = "textDocument/definition")]
    #[serde(rename_all = "camelCase")]
    TextDocumentDefinition {
        position: LsTypePosition,
        text_document: LsTypeTextDocument,
    },
    #[serde(untagged)]
    Unknown {
        method: String,
        params: Option<serde_json::Value>,
    },
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
enum LsType {
    Null,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
enum LSMessageResponseBody {
    Initialize(LSMessageResponseInitialize),
    Location(LSMessageResponseLocation),
    RawType(LsType),
    Shutdown,
}

#[derive(Serialize, Deserialize, Debug)]
struct LSInfo {
    name: String,
    version: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct LsTypeRange {
    start: LsTypePosition,
    end: LsTypePosition,
}

impl LsTypeRange {
    fn range(start: (usize, usize), end: (usize, usize)) -> Self {
        Self {
            start: LsTypePosition {
                line: start.0 as u32,
                character: start.1 as u32,
            },
            end: LsTypePosition {
                line: end.0 as u32,
                character: end.1 as u32,
            },
        }
    }
    // fn beginning() -> Self {
    //     Self {
    //         start: LsTypePosition {
    //             character: 0,
    //             line: 0,
    //         },
    //         end: LsTypePosition {
    //             character: 0,
    //             line: 0,
    //         },
    //     }
    // }
}

#[derive(Serialize, Deserialize, Debug)]
struct LSMessageResponseLocation {
    uri: String,
    range: LsTypeRange,
}

impl LSMessageResponseLocation {
    fn new(uri: String, range: LsTypeRange) -> Self {
        Self { uri, range }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct LSMessageResponseInitialize {
    capabilities: serde_json::Value,
    server_info: LSInfo,
}

impl LSMessageResponseInitialize {
    fn new(name: &str, version: &str, _capabilities: LSClientCapabilities) -> Self {
        let server_capabilities = serde_json::json!({
            "definitionProvider": true
        });
        Self {
            capabilities: server_capabilities,
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

            if buf.len() == 0 {
                break;
            }
            if buf == "\r\n" {
                break;
            }
            let (name, value) = buf.split_once(":").ok_or_else(|| ParseError::Header)?;
            debug!("got header: '{:?}': '{:?}'", name, value);
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
        // debug!("content-raw: {}", content);
        let content: LSMessage = serde_json::from_str(&content)
            .map_err(|e| ParseError::JsonParsing((e, content.to_string())))?;
        debug!("content: {:?}", content);

        Ok(content)
    }

    fn respond_with_error(&self, response: LSMessageError) {
        let response = serde_json::to_string(&response).unwrap();
        let content_length = response.len();
        let response = format!(
            "Content-Length: {content_length}\r\nContent-Length: {content_length}\r\n\r\n{response}"
        );
        debug!("respond with error: {:?}", response);
        println!("{}", response)
    }

    fn respond(&self, response: &LSMessageResponse) {
        let response = serde_json::to_string(&response).unwrap();
        let content_length = response.len();
        let response = format!(
            "Content-Length: {content_length}\r\nContent-Length: {content_length}\r\n\r\n{response}"
        );
        debug!("respond: {:?}", response);
        println!("{}", response)
    }

    fn message_response(&self, request: LSMessageRequestBody) -> LSResult<LSMessageResponseBody> {
        match request {
            LSMessageRequestBody::Initialize { capabilities } => {
                Ok(LSMessageResponseBody::Initialize(
                    LSMessageResponseInitialize::new("myls", "0.0.1", capabilities),
                ))
            }
            LSMessageRequestBody::TextDocumentDefinition {
                position,
                text_document,
            } => {
                debug!(
                    "textDocument/definition recieved at position {position:?} in file: '{}'",
                    text_document.uri
                );
                let uri = text_document.uri;
                let controller_uris = self.get_controller_possible_uris(&uri);
                if controller_uris.is_empty() {
                    return Ok(LSMessageResponseBody::RawType(LsType::Null));
                }
                let file_path = self.path_from_uri(&uri)?;
                let html_contents =
                    fs::read_to_string(file_path).map_err(|e| LSError::InvalidRequest {
                        message: format!("Couldn't read HTML: {e}"),
                    })?;
                let ts_contents = self.get_first_opening_file(controller_uris);
                if ts_contents.is_none() {
                    return Ok(LSMessageResponseBody::RawType(LsType::Null));
                }
                let (ts_file_uri, ts_contents) = ts_contents.unwrap();
                debug!("TS URI: {ts_file_uri},TS contents: {ts_contents}");
                let mut parser = tree_sitter::Parser::new();
                parser
                    .set_language(&tree_sitter_html::LANGUAGE.into())
                    .map_err(LSError::internal)?;
                let tree = parser.parse(&html_contents, None).ok_or_else(|| {
                    LSError::ParseError(ParseError::DocumentParsing { file: uri.clone() })
                })?;

                let mut cursor = tree.walk();
                // cursor.node();
                while let Some(_child_index) = cursor.goto_first_child_for_point(
                    tree_sitter::Point::new(position.line as usize, position.character as usize),
                ) {}
                let node = cursor.node();
                let text = node.utf8_text(&html_contents.as_bytes()).map_err(|_e| {
                    LSError::ParseError(ParseError::DocumentParsing { file: uri.clone() })
                })?;
                let start_column = node.start_position().column;
                let cursor_at = position.character as usize - start_column;
                debug!("cursor is at {cursor_at}: '{}'", &text[cursor_at..]);
                debug!("node={node:?}");
                let mut js_parser = tree_sitter::Parser::new();
                js_parser
                    .set_language(&tree_sitter_javascript::LANGUAGE.into())
                    .map_err(LSError::internal)?;
                let tree = js_parser.parse(text, None).ok_or_else(|| {
                    LSError::ParseError(ParseError::DocumentParsing { file: uri.clone() })
                })?;
                let sexp = tree.root_node().to_sexp();
                debug!("sexp={sexp}");
                let query_controller_exp = r#"
                (member_expression
                    object: (identifier) @obj (#eq? @obj "vm")
                    property: (property_identifier) @method
                )"#;
                let query = Query::new(
                    &tree_sitter_javascript::LANGUAGE.into(),
                    query_controller_exp,
                );
                if let Err(err) = query {
                    debug!("qyery error {err:?}");
                    return Ok(LSMessageResponseBody::RawType(LsType::Null));
                }
                let query = query.unwrap();
                let mut cursor = QueryCursor::new();
                let mut matches = cursor.matches(&query, tree.root_node(), text.as_bytes());
                while let Some(m) = matches.next() {
                    // let obj_name = m.captures[0]
                    //     .node
                    //     .utf8_text(text.as_bytes())
                    //     .map_err(|_e| {
                    //         LSError::ParseError(ParseError::DocumentParsing { file: uri.clone() })
                    //     })?;
                    let prop_name =
                        m.captures[1]
                            .node
                            .utf8_text(text.as_bytes())
                            .map_err(|_e| {
                                LSError::ParseError(ParseError::DocumentParsing {
                                    file: uri.clone(),
                                })
                            })?;
                    // if obj_name == "vm" {
                    debug!("found vm with prop={prop_name}");
                    let mut parser = tree_sitter::Parser::new();
                    let _ =
                        parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into());
                    let tree = parser.parse(&ts_contents, None);
                    if tree.is_none() {
                        continue;
                    }
                    let tree = tree.unwrap();
                    let sexp = tree.root_node().to_sexp();
                    debug!("ts sexp={sexp:?}");
                    let query_field_def = format!(
                        "
                        (
                            public_field_definition
                                name: (property_identifier) @prop
                        )
                       "
                    );
                    debug!("query={query_field_def}");
                    let query = Query::new(
                        &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                        &query_field_def,
                    );
                    if let Err(err) = query {
                        debug!("TS query error: {err}");
                        return Ok(LSMessageResponseBody::RawType(LsType::Null));
                    }
                    let query = query.unwrap();
                    let mut cursor = QueryCursor::new();
                    let mut matches =
                        cursor.matches(&query, tree.root_node(), ts_contents.as_bytes());
                    while let Some(m) = matches.next() {
                        debug!("processing match");
                        let node = m.captures[0].node;
                        let start = node.start_position();
                        let end = node.start_position();
                        return Ok(LSMessageResponseBody::Location(
                            LSMessageResponseLocation::new(
                                ts_file_uri.to_string(),
                                LsTypeRange::range(
                                    (start.row, start.column),
                                    (end.row, end.column),
                                ),
                            ),
                        ));
                    }
                    // }
                }
                // debug!("method={method:?}");
                // let query_extract_member_var = r#"
                //     (member_expression {object = })
                //     "#;
                // let cursor = tree.walk();
                Ok(LSMessageResponseBody::RawType(LsType::Null))
                // Ok(LSMessageResponseBody::Location(
                //     LSMessageResponseLocation::new(uri, LsTypeRange::beginning()),
                // ))
            }
            LSMessageRequestBody::Shutdown => Ok(LSMessageResponseBody::Shutdown),
            LSMessageRequestBody::Unknown { method, params } => {
                debug!("Unknown request: {}. params={:?}", method, params);
                Err(LSError::MethodNotFound(method))
            }
        }
    }

    fn get_first_opening_file<'a>(&self, uris: Vec<String>) -> Option<(String, String)> {
        for (uri, path) in uris
            .iter()
            .map(|uri| self.path_from_uri(uri).map(|path| (uri, path)))
            .flatten()
        {
            if let Ok(contents) = fs::read_to_string(path) {
                return Some((uri.clone(), contents));
            }
        }
        return None;
    }

    fn get_controller_possible_uris(&self, uri: &str) -> Vec<String> {
        uri.strip_suffix(".html")
            .and_then(|uri| uri.rsplit_once("/"))
            .map(|(pre, filename)| {
                let pascalified = filename
                    .split("-")
                    .map(|part| {
                        let mut chars = part.chars();
                        if let Some(first_char) = chars.next() {
                            let rest: String = chars.collect();
                            format!("{}{}", first_char.to_uppercase(), rest)
                        } else {
                            part.to_string()
                        }
                    })
                    .collect::<String>();
                ["Controller", "Directive", ""]
                    .map(|ending| format!("{pre}/{pascalified}{ending}.ts",))
                    .to_vec()
            })
            .unwrap_or(vec![])
    }

    fn path_from_uri<'a>(&self, uri: &'a str) -> LSResult<&'a str> {
        if let Some(path) = uri.strip_prefix("file://") {
            Ok(path)
        } else {
            todo!()
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
    #[error("Internal error: {message}")]
    InternalError { message: String },
    #[error("Invalid request: {message}")]
    InvalidRequest { message: String },
    #[error("Method not found: '{0}'")]
    MethodNotFound(String),
    #[error("Parsing error: '{0}'")]
    ParseError(ParseError),
}

impl LSError {
    fn code(&self) -> i32 {
        match self {
            LSError::InternalError { message: _ } => -32603,
            LSError::InvalidRequest { message: _ } => -32600,
            LSError::MethodNotFound(_) => -32601,
            LSError::ParseError(_) => -32700,
        }
    }
    fn message(&self) -> String {
        self.to_string()
    }

    fn internal<E: std::error::Error>(e: E) -> Self {
        Self::InternalError {
            message: format!("{e}"),
        }
    }
}

#[derive(Error, Debug)]
enum ParseError {
    #[error("Couldn't parse '{file}'")]
    DocumentParsing { file: String },
    #[error("Header invalid")]
    Header,
    #[error("IO error while parsing")]
    Io(#[from] io::Error),
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
