use std::sync::OnceLock;
use tokio::sync::mpsc;
use tower_lsp::Client;
use tower_lsp::lsp_types::WorkDoneProgressCreateParams;
use tower_lsp::lsp_types::request::WorkDoneProgressCreate;
use tower_lsp::lsp_types::{
    MessageType, NumberOrString, ProgressParams, ProgressParamsValue, WorkDoneProgress,
    WorkDoneProgressBegin, WorkDoneProgressEnd, WorkDoneProgressReport, notification::Progress,
};
use tracing::debug;

#[derive(Debug, Clone)]
pub struct LogMessage {
    pub message_type: MessageType,
    pub content: String,
}

impl LogMessage {
    pub fn info(content: impl Into<String>) -> Self {
        Self {
            message_type: MessageType::INFO,
            content: content.into(),
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            message_type: MessageType::ERROR,
            content: content.into(),
        }
    }

    pub fn warning(content: impl Into<String>) -> Self {
        Self {
            message_type: MessageType::WARNING,
            content: content.into(),
        }
    }

    pub fn progress(token: &str, message: &str, percent: f32) -> Self {
        Self {
            message_type: MessageType::INFO,
            content: format!("{}\x1F{}\x1F{:.1}", token, message, percent),
        }
    }

    pub fn progress_begin(token: &str, title: &str) -> Self {
        Self {
            message_type: MessageType::INFO,
            content: format!("BEGIN\x1F{}\x1F{}", token, title),
        }
    }

    pub fn progress_end(token: &str) -> Self {
        Self {
            message_type: MessageType::INFO,
            content: format!("END\x1F{}", token),
        }
    }
}

pub struct LoggingService {
    sender: mpsc::Sender<LogMessage>,
}

impl LoggingService {
    pub fn new(client: Client) -> Self {
        let (sender, receiver) = mpsc::channel(1000);

        tokio::spawn(Self::message_handler(client, receiver));

        Self { sender }
    }

    pub fn send(&self, message: LogMessage) {
        let _ = self.sender.try_send(message);
    }

    async fn message_handler(client: Client, mut receiver: mpsc::Receiver<LogMessage>) {
        while let Some(message) = receiver.recv().await {
            if message.message_type == MessageType::INFO && message.content.contains('\x1F') {
                let parts: Vec<&str> = message.content.split('\x1F').collect();

                if parts[0] == "BEGIN" && parts.len() == 3 {
                    let token = NumberOrString::String(parts[1].to_string());

                    let create_params = WorkDoneProgressCreateParams {
                        token: token.clone(),
                    };

                    let _ = client
                        .send_request::<WorkDoneProgressCreate>(create_params)
                        .await;

                    let _ = client
                        .send_notification::<Progress>(ProgressParams {
                            token: NumberOrString::String(parts[1].to_string()),
                            value: ProgressParamsValue::WorkDone(WorkDoneProgress::Begin(
                                WorkDoneProgressBegin {
                                    title: parts[2].to_string(),
                                    percentage: Some(0),
                                    ..Default::default()
                                },
                            )),
                        })
                        .await;
                } else if parts[0] == "END" && parts.len() == 2 {
                    let _ = client
                        .send_notification::<Progress>(ProgressParams {
                            token: NumberOrString::String(parts[1].to_string()),
                            value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(
                                WorkDoneProgressEnd { message: None },
                            )),
                        })
                        .await;
                } else if parts.len() == 3 {
                    if let Ok(percent) = parts[2].parse::<f32>() {
                        let _ = client
                            .send_notification::<Progress>(ProgressParams {
                                token: NumberOrString::String(parts[0].to_string()),
                                value: ProgressParamsValue::WorkDone(WorkDoneProgress::Report(
                                    WorkDoneProgressReport {
                                        percentage: Some(percent as u32),
                                        message: Some(parts[1].to_string()),
                                        ..Default::default()
                                    },
                                )),
                            })
                            .await;
                    }
                }
            } else {
                client
                    .show_message(message.message_type, &message.content)
                    .await;
            }
        }
    }
}

static LOGGING_SERVICE: OnceLock<LoggingService> = OnceLock::new();

pub fn init_logging_service(client: Client) {
    let service = LoggingService::new(client);
    let _ = LOGGING_SERVICE.set(service);
}

pub fn log_message(message: LogMessage) {
    debug!("{}", message.content.clone());
    if let Some(service) = LOGGING_SERVICE.get() {
        service.send(message);
    }
}

#[macro_export]
macro_rules! lsp_info {
    ($($arg:tt)*) => {
        $crate::lsp_logging::log_message(
            $crate::lsp_logging::LogMessage::info(format!($($arg)*))
        )
    };
}

#[macro_export]
macro_rules! lsp_error {
    ($($arg:tt)*) => {
        $crate::lsp_logging::log_message(
            $crate::lsp_logging::LogMessage::error(format!($($arg)*))
        )
    };
}

#[macro_export]
macro_rules! lsp_warn {
    ($($arg:tt)*) => {
        $crate::lsp_logging::log_message(
            $crate::lsp_logging::LogMessage::warning(format!($($arg)*))
        )
    };
}

#[macro_export]
macro_rules! lsp_debug {
    ($($arg:tt)*) => {
        $crate::lsp_logging::log_message(
            $crate::lsp_logging::LogMessage::log(format!($($arg)*))
        )
    };
}

#[macro_export]
macro_rules! lsp_progress {
    ($token:expr, $message:expr, $percent:expr) => {
        $crate::lsp_logging::log_message($crate::lsp_logging::LogMessage::progress(
            $token, $message, $percent,
        ))
    };
}

#[macro_export]
macro_rules! lsp_progress_begin {
    ($token:expr, $title:expr) => {
        $crate::lsp_logging::log_message($crate::lsp_logging::LogMessage::progress_begin(
            $token, $title,
        ))
    };
}

#[macro_export]
macro_rules! lsp_progress_end {
    ($token:expr) => {
        $crate::lsp_logging::log_message($crate::lsp_logging::LogMessage::progress_end($token))
    };
}
