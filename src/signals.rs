//! D-Bus PropertiesChanged invalidations shared by the Mako control API
//! and the Freedesktop notification receiver.

use std::collections::HashMap;
use zbus::{zvariant::OwnedValue, Connection};

use crate::control::{CONTROL_INTERFACE, CONTROL_PATH};

pub async fn invalidate(conn: &Connection, property: &str) -> zbus::Result<()> {
    // Mako 1.11.0 invalidates properties instead of sending their entire value.
    let changed = HashMap::<String, OwnedValue>::new();
    let invalidated = vec![property];
    conn.emit_signal(None::<&str>, CONTROL_PATH,
        "org.freedesktop.DBus.Properties", "PropertiesChanged",
        &(CONTROL_INTERFACE, changed, invalidated))
        .await
}
