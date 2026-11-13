//! IndexedDB backend: an in-memory mirror filled at startup, with writes
//! persisted in the background.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::Uint8Array;
use wasm_bindgen::prelude::*;
use web_sys::{IdbCursorWithValue, IdbDatabase, IdbOpenDbRequest, IdbRequest, IdbTransactionMode};

use crate::Memory;

const DATABASE: &str = "araria";
const STORE: &str = "files";

#[derive(Clone)]
pub(crate) struct WebStore {
    memory: Rc<RefCell<Memory>>,
    db: Rc<RefCell<Option<IdbDatabase>>>,
}

fn describe(error: &JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}

impl WebStore {
    pub(crate) fn open() -> Self {
        let store = WebStore { memory: Rc::new(RefCell::new(Memory::default())), db: Rc::new(RefCell::new(None)) };
        if let Err(e) = store.start() {
            store.give_up(&describe(&e));
        }
        store
    }

    pub(crate) fn memory(&self) -> &Rc<RefCell<Memory>> {
        &self.memory
    }

    /// Falls back to memory-only operation.
    fn give_up(&self, reason: &str) {
        let mut memory = self.memory.borrow_mut();
        memory.ready = true;
        memory.warning = Some(format!("Browser storage is unavailable ({reason}); changes will be lost when this tab closes."));
    }

    fn start(&self) -> Result<(), JsValue> {
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
        let factory = window.indexed_db()?.ok_or_else(|| JsValue::from_str("IndexedDB is disabled"))?;
        let request: IdbOpenDbRequest = factory.open_with_u32(DATABASE, 1)?;

        let upgrade_request = request.clone();
        let on_upgrade = Closure::once_into_js(move || {
            if let Ok(db) = upgrade_request.result().and_then(|r| r.dyn_into::<IdbDatabase>()) {
                let _ = db.create_object_store(STORE);
            }
        });
        request.set_onupgradeneeded(Some(on_upgrade.unchecked_ref()));

        let (this, opened) = (self.clone(), request.clone());
        let on_success = Closure::once_into_js(move || match opened.result().and_then(|r| r.dyn_into::<IdbDatabase>()) {
            Ok(db) => {
                *this.db.borrow_mut() = Some(db.clone());
                if let Err(e) = this.load_all(&db) {
                    this.give_up(&describe(&e));
                }
            }
            Err(e) => this.give_up(&describe(&e)),
        });
        request.set_onsuccess(Some(on_success.unchecked_ref()));

        let this = self.clone();
        let on_error = Closure::once_into_js(move || this.give_up("the database could not be opened"));
        request.set_onerror(Some(on_error.unchecked_ref()));
        Ok(())
    }

    /// Walks every record with a cursor and copies it into memory.
    fn load_all(&self, db: &IdbDatabase) -> Result<(), JsValue> {
        let store = db.transaction_with_str(STORE)?.object_store(STORE)?;
        let request: IdbRequest = store.open_cursor()?;
        let (this, cursor_request) = (self.clone(), request.clone());
        // The cursor callback fires once per record, so it must stay alive
        // for the page's lifetime; it is created once per session.
        let on_step = Closure::<dyn FnMut()>::new(move || {
            let cursor = cursor_request.result().ok().and_then(|r| r.dyn_into::<IdbCursorWithValue>().ok());
            match cursor {
                Some(cursor) => {
                    if let (Some(key), Ok(value)) = (cursor.key().ok().and_then(|k| k.as_string()), cursor.value()) {
                        // Files written while loading win over stored copies.
                        this.memory.borrow_mut().files.entry(key).or_insert_with(|| Uint8Array::new(&value).to_vec());
                    }
                    let _ = cursor.continue_();
                }
                None => this.memory.borrow_mut().ready = true,
            }
        });
        request.set_onsuccess(Some(on_step.as_ref().unchecked_ref()));
        on_step.forget();
        Ok(())
    }

    /// Writes (`Some`) or deletes (`None`) one record in the background.
    pub(crate) fn persist(&self, path: &str, data: Option<&[u8]>) {
        let db = self.db.borrow();
        let Some(db) = db.as_ref() else { return };
        let result = db
            .transaction_with_str_and_mode(STORE, IdbTransactionMode::Readwrite)
            .and_then(|tx| tx.object_store(STORE))
            .and_then(|store| match data {
                Some(bytes) => store.put_with_key(&Uint8Array::from(bytes), &JsValue::from_str(path)),
                None => store.delete(&JsValue::from_str(path)),
            });
        if let Err(e) = result {
            log::error!("could not persist {path}: {}", describe(&e));
        }
    }
}
