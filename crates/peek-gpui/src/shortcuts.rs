//! Register shortcuts as a transaction; failed changes keep the old bindings.
use global_hotkey::{GlobalHotKeyManager, hotkey::HotKey};
use std::cell::RefCell;
thread_local! {
    static ACTIVE: RefCell<Option<(GlobalHotKeyManager, Vec<HotKey>)>> = const { RefCell::new(None) };
}
pub fn apply(keys: Vec<HotKey>) -> Result<(), String> {
    ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        if active.is_none() {
            *active = Some((
                GlobalHotKeyManager::new()
                    .map_err(|_| peek_runtime::i18n::tr("status-hotkey-manager"))?,
                Vec::new(),
            ));
        }
        let (manager, old) = active.as_mut().unwrap();
        manager
            .unregister_all(old)
            .map_err(|_| peek_runtime::i18n::tr("status-hotkey-manager"))?;
        let mut registered = Vec::new();
        for key in &keys {
            if manager.register(*key).is_err() {
                let _ = manager.unregister_all(&registered);
                for previous in old.iter() {
                    let _ = manager.register(*previous);
                }
                return Err(peek_runtime::i18n::tr("status-hotkey-conflict"));
            }
            registered.push(*key);
        }
        *old = keys;
        Ok(())
    })
}
pub fn role(id: u32) -> Option<usize> {
    ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()
            .and_then(|(_, keys)| keys.iter().position(|key| key.id() == id))
    })
}
