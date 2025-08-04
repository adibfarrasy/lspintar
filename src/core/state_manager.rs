use serde_json::Value;
use std::sync::{OnceLock, RwLock};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, oneshot};

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

static STATE_STORE: OnceLock<Arc<RwLock<HashMap<String, Value>>>> = OnceLock::new();

pub fn init_state_manager() {
    let store = Arc::new(RwLock::new(HashMap::new()));
    let _ = STATE_STORE.set(store);
}

pub fn set_global(key: impl Into<String>, value: impl Into<Value>) {
    if let Some(store) = STATE_STORE.get() {
        if let Ok(mut map) = store.write() {
            map.insert(key.into(), value.into());
        }
    }
}

pub fn get_global(key: impl Into<String>) -> Option<Value> {
    STATE_STORE
        .get()
        .and_then(|store| store.read().ok())
        .and_then(|map| map.get(&key.into()).cloned())
}
