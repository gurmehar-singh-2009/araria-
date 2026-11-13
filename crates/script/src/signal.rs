//! Events: `signal:Connect(fn)`, `:Once`, `:Wait()` and connections.

use std::any::Any;
use std::rc::Rc;

use araria_lua::{Error, Lua, Result, Ret, UserData, Value, func_arg};

use crate::host::{Connection, Host, SignalKey};

/// An event that scripts can listen to.
pub struct Signal {
    pub key: SignalKey,
    pub host: Rc<Host>,
    /// Whether scripts may fire it themselves (`Signal.new()`).
    pub custom: bool,
}

pub fn signal_value(host: &Rc<Host>, id: u64, name: &str, custom: bool) -> Value {
    Lua::userdata(Signal { key: (id, Rc::from(name)), host: host.clone(), custom })
}

impl UserData for Signal {
    fn type_name(&self) -> &'static str {
        "Signal"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn index(&self, _lua: &mut Lua, key: &Value) -> Result<Value> {
        let methods = self.host.signal_methods.borrow();
        let value = methods.as_ref().map_or(Value::Nil, |t| t.get(key));
        if value.is_nil() {
            return Err(Error::runtime(format!("{} is not a valid member of Signal", key.to_display())));
        }
        Ok(value)
    }
    fn to_string(&self) -> String {
        format!("Signal {}", self.key.1)
    }
}

/// The handle returned by `Connect`.
pub struct ConnectionHandle {
    key: SignalKey,
    id: u64,
    host: Rc<Host>,
}

impl ConnectionHandle {
    fn connected(&self) -> bool {
        self.host.state.borrow().connections.get(&self.key).is_some_and(|l| l.iter().any(|c| c.id == self.id))
    }
}

impl UserData for ConnectionHandle {
    fn type_name(&self) -> &'static str {
        "Connection"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn index(&self, _lua: &mut Lua, key: &Value) -> Result<Value> {
        match key.as_str().as_deref() {
            Some("Connected") => Ok(Value::Bool(self.connected())),
            Some("Disconnect") => {
                let (host, signal, id) = (self.host.clone(), self.key.clone(), self.id);
                Ok(Lua::native("Connection.Disconnect", move |_, _| {
                    if let Some(list) = host.state.borrow_mut().connections.get_mut(&signal) {
                        list.retain(|c| c.id != id);
                    }
                    Ok(Ret::None)
                }))
            }
            _ => Err(Error::runtime(format!("{} is not a valid member of Connection", key.to_display()))),
        }
    }
}

fn self_signal(args: &[Value], method: &str) -> Result<(SignalKey, Rc<Host>, bool)> {
    args.first()
        .and_then(|v| v.as_userdata::<Signal>())
        .map(|s| (s.key.clone(), s.host.clone(), s.custom))
        .ok_or_else(|| Error::runtime(format!("{method} must be called with ':' on a Signal")))
}

fn connect(args: &[Value], once: bool) -> Result<Ret> {
    let (key, host, _) = self_signal(args, "Connect")?;
    let func = func_arg(args, 1, "Connect")?;
    let id = host.next_id();
    let script = host.current_script();
    // Instances with listeners report property changes.
    if &*key.1 == "Changed" || key.1.starts_with("Changed:") {
        host.dm.borrow_mut().watch(araria_core::InstanceId(key.0));
    }
    host.state.borrow_mut().connections.entry(key.clone()).or_default().push(Connection { id, func, script, once });
    Ok(Ret::One(Lua::userdata(ConnectionHandle { key, id, host })))
}

/// Builds the table of methods shared by every signal.
pub fn register(lua: &mut Lua, host: &Rc<Host>) {
    let methods = lua.new_table();
    methods.set_str("Connect", Lua::native("Signal.Connect", |_, args| connect(args, false)));
    methods.set_str("Once", Lua::native("Signal.Once", |_, args| connect(args, true)));
    methods.set_str(
        "Wait",
        Lua::native("Signal.Wait", |lua, args| {
            let (key, host, _) = self_signal(args, "Wait")?;
            let Some(thread) = lua.current_thread() else {
                return Err(Error::runtime("Signal:Wait can only be used inside a script"));
            };
            if &*key.1 == "Changed" || key.1.starts_with("Changed:") {
                host.dm.borrow_mut().watch(araria_core::InstanceId(key.0));
            }
            let script = host.current_script();
            host.state.borrow_mut().waiters.entry(key).or_default().push((thread, script));
            Ok(Ret::Yield(Vec::new()))
        }),
    );
    methods.set_str(
        "Fire",
        Lua::native("Signal.Fire", |lua, args| {
            let (key, host, custom) = self_signal(args, "Fire")?;
            if !custom {
                return Err(Error::runtime(format!("{} is fired by the engine and cannot be fired from a script", key.1)));
            }
            host.fire(lua, &key, &args[1..]);
            Ok(Ret::None)
        }),
    );
    methods.set_str(
        "DisconnectAll",
        Lua::native("Signal.DisconnectAll", |_, args| {
            let (key, host, _) = self_signal(args, "DisconnectAll")?;
            host.state.borrow_mut().connections.remove(&key);
            Ok(Ret::None)
        }),
    );
    *host.signal_methods.borrow_mut() = Some(methods);

    // `Signal.new()` creates an event scripts can fire themselves.
    let signal = lua.new_table();
    let h = host.clone();
    signal.set_str(
        "new",
        Lua::native("Signal.new", move |_, _| {
            let id = (1u64 << 60) | h.next_id();
            Ok(Ret::One(signal_value(&h, id, "Signal", true)))
        }),
    );
    lua.set_global("Signal", signal);
}
