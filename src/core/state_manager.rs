use serde_json::Value;
use std::sync::{OnceLock, RwLock};
use std::{collections::HashMap, sync::Arc};


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
