use std::sync::OnceLock;
use tokio::sync::mpsc;
use tower_lsp::lsp_types::MessageType;
use tower_lsp::Client;
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

    pub fn log(content: impl Into<String>) -> Self {
        Self {
            message_type: MessageType::LOG,
            content: content.into(),
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
        // Fire-and-forget - ignore if channel is full or closed
        let _ = self.sender.try_send(message);
    }

    async fn message_handler(client: Client, mut receiver: mpsc::Receiver<LogMessage>) {
        while let Some(message) = receiver.recv().await {
            client
                .show_message(message.message_type, &message.content)
                .await;
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
        $crate::core::logging_service::log_message(
            $crate::core::logging_service::LogMessage::info(format!($($arg)*))
        )
    };
}

#[macro_export]
macro_rules! lsp_error {
    ($($arg:tt)*) => {
        $crate::core::logging_service::log_message(
            $crate::core::logging_service::LogMessage::error(format!($($arg)*))
        )
    };
}

#[macro_export]
macro_rules! lsp_warning {
    ($($arg:tt)*) => {
        $crate::core::logging_service::log_message(
            $crate::core::logging_service::LogMessage::warning(format!($($arg)*))
        )
    };
}

#[macro_export]
macro_rules! lsp_debug {
    ($($arg:tt)*) => {
        $crate::core::logging_service::log_message(
            $crate::core::logging_service::LogMessage::log(format!($($arg)*))
        )
    };
}
