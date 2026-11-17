//! Correlated, bounded control replies, consumed before transcript handling.
use crate::{runtime::BridgeCore, BridgeError};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex, OnceLock},
    time::Duration,
};

struct Pending {
    session: String,
    sender: mpsc::SyncSender<Result<Value, String>>,
}
fn pending() -> &'static Mutex<HashMap<String, Pending>> {
    static PENDING: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    PENDING.get_or_init(Mutex::default)
}
struct Registration(String);
impl Drop for Registration {
    fn drop(&mut self) {
        pending()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.0);
    }
}

pub fn request(
    core: &Arc<BridgeCore>,
    session: &str,
    request: &Value,
) -> Result<Value, BridgeError> {
    request_with(session, |id| {
        let adapters = core
            .adapters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let runtime = adapters.get(session).ok_or_else(|| {
            BridgeError::Invalid("Resume this chat to reconnect its MCP servers".into())
        })?;
        runtime.app_control(id, request)
    })
}

/// One correlated request to whatever process `owner` names: a chat's live
/// runtime or a warm connection runtime. `send` writes the request carrying
/// the id; the owner's reader hands the reply to [`deliver`] under the same
/// owner, so a reply can never cross from one process to another.
pub fn request_with(
    owner: &str,
    send: impl FnOnce(&str) -> Result<(), BridgeError>,
) -> Result<Value, BridgeError> {
    let id = uuid::Uuid::new_v4().to_string();
    let (sender, receiver) = mpsc::sync_channel(1);
    {
        let mut all = pending()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if all.len() >= 64 || all.values().filter(|p| p.session == owner).count() >= 4 {
            return Err(BridgeError::Invalid(
                "Too many app preview requests; try again shortly".into(),
            ));
        }
        all.insert(
            id.clone(),
            Pending {
                session: owner.into(),
                sender,
            },
        );
    }
    let _registration = Registration(id.clone());
    send(&id)?;
    receiver
        .recv_timeout(Duration::from_secs(30))
        .map_err(|_| {
            BridgeError::Invalid(
                "The app preview timed out; check this chat's MCP connection".into(),
            )
        })?
        .map_err(BridgeError::Invalid)
}

/// Always consume control frames, even unknown/stale ones; never persist them.
pub fn deliver(session: &str, frame: &Value) -> bool {
    if frame["type"] != "app_control_result" {
        return false;
    }
    let Some(id) = frame["id"].as_str() else {
        return true;
    };
    let mut all = pending()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if all.get(id).is_none_or(|p| p.session != session) {
        return true;
    }
    if let Some(p) = all.remove(id) {
        let result = if frame["ok"] == true {
            let value = frame["result"].clone();
            if value.to_string().len() > 256 * 1024 {
                Err("The preview result exceeds 256 KiB".into())
            } else {
                Ok(value)
            }
        } else {
            Err("The MCP preview request failed; check the connection in this chat".into())
        };
        drop(p.sender.try_send(result));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn replies_are_session_bound_and_consumed_once() {
        let id = uuid::Uuid::new_v4().to_string();
        let (sender, receiver) = mpsc::sync_channel(1);
        pending().lock().unwrap().insert(
            id.clone(),
            Pending {
                session: "owner".into(),
                sender,
            },
        );
        let frame = json!({"type":"app_control_result","id":id,"ok":true,"result":{"secret":"read result"}});
        assert!(deliver("other", &frame));
        assert!(receiver.try_recv().is_err());
        assert!(deliver("owner", &frame));
        assert_eq!(receiver.recv().unwrap().unwrap(), frame["result"]);
        assert!(deliver("owner", &frame));
        assert!(!deliver("owner", &json!({"type":"assistant"})));
    }
    #[test]
    fn dropped_registration_removes_timed_out_request() {
        let id = uuid::Uuid::new_v4().to_string();
        let (sender, _) = mpsc::sync_channel(1);
        pending().lock().unwrap().insert(
            id.clone(),
            Pending {
                session: "timeout".into(),
                sender,
            },
        );
        drop(Registration(id.clone()));
        assert!(!pending().lock().unwrap().contains_key(&id));
    }
}
