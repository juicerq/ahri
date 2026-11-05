use smithay_client_toolkit::reexports::calloop::channel::Sender;
use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Type;
use zbus::zvariant::OwnedObjectPath;

use crate::Msg;

const COMPONENT: &str = "ahri";
const COMPONENT_NAME: &str = "Ahri";
const META: i32 = 0x1000_0000;
const SHIFT: i32 = 0x0200_0000;
const KEY_R: i32 = 0x52;
const KEY_S: i32 = 0x53;
const SET_PRESENT_NO_AUTOLOADING: u32 = 2 | 4;
const TAKEOVER_FROM: &str = "org.kde.spectacle.desktop";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Screenshot,
    Record,
}

const ACTIONS: [(Action, &str, &str, i32); 2] = [
    (
        Action::Screenshot,
        "screenshot",
        "Capturar região",
        META | SHIFT | KEY_S,
    ),
    (Action::Record, "record", "Gravar região", META | KEY_R),
];

pub fn spawn(tx: Sender<Msg>) {
    std::thread::spawn(move || {
        if let Err(e) = run(&tx) {
            eprintln!("ahri: shortcuts: {e}");
            let _ = tx.send(Msg::Fatal(format!("shortcuts: {e}")));
        }
    });
}

fn run(tx: &Sender<Msg>) -> zbus::Result<()> {
    let conn = Connection::session()?;

    for (_, name, friendly, key) in ACTIONS {
        register(&conn, name, friendly, key)?;
    }

    let path: OwnedObjectPath = call(&conn, "getComponent", &(COMPONENT,))?.body().deserialize()?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.kde.kglobalaccel.Component")?
        .member("globalShortcutPressed")?
        .path(path)?
        .build();

    for msg in MessageIterator::for_match_rule(rule, &conn, None)? {
        let msg = msg?;
        let Ok((component, name, _ts)) = msg.body().deserialize::<(String, String, i64)>() else {
            continue;
        };
        if component != COMPONENT {
            continue;
        }
        if let Some((action, ..)) = ACTIONS.iter().find(|(_, n, ..)| *n == name)
            && tx.send(Msg::Shortcut(*action)).is_err()
        {
            break;
        }
    }
    Ok(())
}

fn register(conn: &Connection, name: &str, friendly: &str, key: i32) -> zbus::Result<()> {
    let id = vec![COMPONENT, name, COMPONENT_NAME, friendly];
    call(conn, "doRegister", &(&id,))?;

    let owner: Vec<String> = call(conn, "action", &(key,))?.body().deserialize()?;
    match owner.first().map(String::as_str) {
        None | Some(COMPONENT) => {}
        Some(TAKEOVER_FROM) => {
            let keys: Vec<i32> = call(conn, "shortcut", &(&owner,))?.body().deserialize()?;
            let keep: Vec<i32> = keys.into_iter().filter(|k| *k != key).collect();
            eprintln!("ahri: taking {friendly} shortcut from {}", owner[..2].join("/"));
            call(conn, "setForeignShortcut", &(&owner, keep))?;
        }
        Some(_) => eprintln!("ahri: {friendly} shortcut is also bound to {}", owner[..2].join("/")),
    }

    if !set(conn, &id, key)? {
        eprintln!("ahri: kglobalaccel refused shortcut for {name}");
    }
    Ok(())
}

fn set(conn: &Connection, id: &[&str], key: i32) -> zbus::Result<bool> {
    let keys: Vec<i32> = call(conn, "setShortcut", &(id, vec![key], SET_PRESENT_NO_AUTOLOADING))?
        .body()
        .deserialize()?;
    Ok(keys == [key])
}

fn call<B>(conn: &Connection, method: &str, body: &B) -> zbus::Result<zbus::Message>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    conn.call_method(
        Some("org.kde.kglobalaccel"),
        "/kglobalaccel",
        Some("org.kde.KGlobalAccel"),
        method,
        body,
    )
}
