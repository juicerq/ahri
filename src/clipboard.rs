use std::io::Write;
use std::sync::Arc;

use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::reexports::client::globals::GlobalList;
use smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat;
use smithay_client_toolkit::reexports::client::{Connection, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
    ext_data_control_source_v1::{self, ExtDataControlSourceV1},
};

use crate::App;

pub struct Clipboard {
    manager: ExtDataControlManagerV1,
    device: Option<ExtDataControlDeviceV1>,
    source: Option<(ExtDataControlSourceV1, Arc<Payload>)>,
}

pub struct Payload {
    pub entries: Vec<(&'static str, Vec<u8>)>,
}

pub struct ClipData;

impl Clipboard {
    pub fn bind(globals: &GlobalList, qh: &QueueHandle<App>) -> Result<Self, String> {
        let manager = globals
            .bind::<ExtDataControlManagerV1, _, _>(qh, 1..=1, ClipData)
            .map_err(|e| format!("ext_data_control_manager_v1: {e}"))?;
        Ok(Self {
            manager,
            device: None,
            source: None,
        })
    }

    pub fn set_seat(&mut self, seat: &WlSeat, qh: &QueueHandle<App>) {
        if self.device.is_none() {
            self.device = Some(self.manager.get_data_device(seat, qh, ClipData));
        }
    }

    pub fn set(&mut self, payload: Payload, qh: &QueueHandle<App>) -> Result<(), String> {
        let device = self.device.as_ref().ok_or("no seat for clipboard")?;
        let source = self.manager.create_data_source(qh, ClipData);
        for (mime, _) in &payload.entries {
            source.offer(mime.to_string());
        }
        device.set_selection(Some(&source));
        if let Some((old, _)) = self.source.replace((source, Arc::new(payload))) {
            old.destroy();
        }
        Ok(())
    }

    fn send(&self, source: &ExtDataControlSourceV1, mime: &str, fd: std::os::fd::OwnedFd) {
        let Some((current, payload)) = &self.source else { return };
        if current != source {
            return;
        }
        let payload = payload.clone();
        let mime = mime.to_string();
        std::thread::spawn(move || {
            let Some((_, bytes)) = payload.entries.iter().find(|(m, _)| *m == mime) else {
                return;
            };
            let mut file = std::fs::File::from(fd);
            if let Err(e) = file.write_all(bytes) {
                eprintln!("ahri: clipboard write {mime}: {e}");
            }
        });
    }

    fn cancelled(&mut self, source: &ExtDataControlSourceV1) {
        if self.source.as_ref().is_some_and(|(s, _)| s == source) {
            self.source = None;
        }
        source.destroy();
    }
}

impl Dispatch2<ExtDataControlManagerV1, App> for ClipData {
    fn event(
        &self,
        _: &mut App,
        _: &ExtDataControlManagerV1,
        _: <ExtDataControlManagerV1 as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}

impl Dispatch2<ExtDataControlDeviceV1, App> for ClipData {
    event_created_child!(App, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, ClipData)
    ]);

    fn event(
        &self,
        app: &mut App,
        _: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        match event {
            ext_data_control_device_v1::Event::Selection { id: Some(offer) }
            | ext_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => offer.destroy(),
            ext_data_control_device_v1::Event::Finished => {
                if let Some(clip) = app.clipboard.as_mut() {
                    clip.device = None;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch2<ExtDataControlOfferV1, App> for ClipData {
    fn event(
        &self,
        _: &mut App,
        _: &ExtDataControlOfferV1,
        _: ext_data_control_offer_v1::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}

impl Dispatch2<ExtDataControlSourceV1, App> for ClipData {
    fn event(
        &self,
        app: &mut App,
        source: &ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        let Some(clip) = app.clipboard.as_mut() else { return };
        match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => clip.send(source, &mime_type, fd),
            ext_data_control_source_v1::Event::Cancelled => clip.cancelled(source),
            _ => {}
        }
    }
}
