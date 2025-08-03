use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::{
    runtime::Handle,
    sync::{mpsc, oneshot},
};

#[derive(Debug)]
pub enum StateCommand {
    Set {
        key: String,
        value: Value,
    },
    Get {
        key: String,
        response: oneshot::Sender<Option<Value>>,
    },
    Delete {
        key: String,
    },
    Clear,
    GetAll {
        response: oneshot::Sender<HashMap<String, Value>>,
    },
}

pub struct StateManager {
    sender: mpsc::Sender<StateCommand>,
}

impl StateManager {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel(1000);

        // Spawn background task to handle state mutations
        tokio::spawn(Self::state_handler(receiver));

        Self { sender }
    }

    pub async fn set(&self, key: impl Into<String>, value: impl Into<Value>) {
        let command = StateCommand::Set {
            key: key.into(),
            value: value.into(),
        };
        let _ = self.sender.send(command).await;
    }

    pub async fn get(&self, key: impl Into<String>) -> Option<Value> {
        let (tx, rx) = oneshot::channel();
        let command = StateCommand::Get {
            key: key.into(),
            response: tx,
        };

        if self.sender.send(command).await.is_ok() {
            rx.await.unwrap_or(None)
        } else {
            None
        }
    }

    pub async fn delete(&self, key: impl Into<String>) {
        let command = StateCommand::Delete { key: key.into() };
        let _ = self.sender.send(command).await;
    }

    async fn state_handler(mut receiver: mpsc::Receiver<StateCommand>) {
        let mut state: HashMap<String, Value> = HashMap::new();

        while let Some(command) = receiver.recv().await {
            match command {
                StateCommand::Set { key, value } => {
                    state.insert(key, value);
                }
                StateCommand::Get { key, response } => {
                    let value = state.get(&key).cloned();
                    let _ = response.send(value);
                }
                StateCommand::Delete { key } => {
                    state.remove(&key);
                }
                StateCommand::Clear => {
                    state.clear();
                }
                StateCommand::GetAll { response } => {
                    let _ = response.send(state.clone());
                }
            }
        }
    }
}

static STATE_MANAGER: OnceLock<StateManager> = OnceLock::new();

pub fn init_state_manager() {
    let manager = StateManager::new();
    let _ = STATE_MANAGER.set(manager);
}

pub async fn set_global(key: impl Into<String>, value: impl Into<Value>) {
    if let Some(manager) = STATE_MANAGER.get() {
        manager.set(key, value).await;
    }
}

pub fn get_global(key: impl Into<String>) -> Option<Value> {
    if let Ok(handle) = Handle::try_current() {
        handle.block_on(async {
            if let Some(manager) = STATE_MANAGER.get() {
                manager.get(key).await
            } else {
                None
            }
        })
    } else {
        // Fallback if no runtime available
        None
    }
}
