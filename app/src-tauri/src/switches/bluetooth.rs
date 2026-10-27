//! Bluetooth Headphones (FR-38, spec 7.5, task 4.10). Experimental: the
//! connect/disconnect is the undocumented `IKsControl`
//! `KSPROPERTY_ONESHOT_RECONNECT/DISCONNECT` on the Bluetooth audio
//! endpoints (ToothTray prior art, D-014), gated by a read-only startup
//! probe. Devices are the render-endpoint containers whose topology walks
//! reach a `{2}.\?\bth…` device (probe, 2026-10-27); names come from the
//! `BTHENUM\DEV_*` node's FriendlyName (fallback: the endpoint name) and
//! the battery from `DEVPKEY{104EA319-…, 2}` (BYTE) on any node of the
//! container — shown only while connected, since it goes stale (probe).

use std::collections::HashMap;
use std::sync::Mutex;
#[cfg(not(test))]
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GETIDLIST_FILTER_ENUMERATOR, CM_Get_DevNode_PropertyW, CM_Get_Device_ID_List_SizeW,
    CM_Get_Device_ID_ListW, CM_LOCATE_DEVNODE_PHANTOM, CM_Locate_DevNodeW, CR_SUCCESS,
};
use windows::Win32::Devices::FunctionDiscovery::{
    PKEY_Device_ContainerId, PKEY_Device_FriendlyName,
};
use windows::Win32::Devices::Properties::{
    DEVPKEY_Device_ContainerId, DEVPKEY_Device_FriendlyName, DEVPROP_TYPE_BYTE, DEVPROP_TYPE_GUID,
    DEVPROP_TYPE_STRING, DEVPROPTYPE,
};
use windows::Win32::Foundation::{DEVPROPKEY, E_INVALIDARG, PROPERTYKEY};
use windows::Win32::Media::Audio::{
    DEVICE_STATE, DEVICE_STATE_ACTIVE, DEVICE_STATEMASK_ALL, IDeviceTopology, IMMDevice,
    IMMDeviceCollection, IMMDeviceEnumerator, IPart, MMDeviceEnumerator, eRender,
};
use windows::Win32::Media::KernelStreaming::{
    IKsControl, KSIDENTIFIER, KSPROPERTY_ONESHOT_DISCONNECT, KSPROPERTY_ONESHOT_RECONNECT,
    KSPROPERTY_TYPE_GET, KSPROPSETID_BtAudio,
};
use windows::Win32::System::Com::StructuredStorage::{
    PropVariantClear, PropVariantToGUID, PropVariantToStringAlloc,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::{GUID, Interface, PCWSTR, PWSTR};

use super::{Availability, Ctx, RestoreOutcome, Switch, SwitchError, SwitchId, parse_state};
use crate::com::ComGuard;
use crate::journal::JournalEntry;
use crate::os::OsInfo;

/// Battery level on any BTHENUM node of the container (BtBatteryTray prior
/// art, U-8 resolved by the probe).
const DEVPKEY_BLUETOOTH_BATTERY: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),
    pid: 2,
};

/// The user's chosen device (settings.bluetooth.deviceId), held here for
/// the switch to read. Set at launch and by set_bluetooth_device.
static CHOSEN: Mutex<Option<String>> = Mutex::new(None);
#[cfg(not(test))]
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

#[cfg(not(test))]
pub fn set_app(app: tauri::AppHandle) {
    let _ = APP.set(app);
}

#[cfg(test)]
pub fn set_app(app: tauri::AppHandle) {
    let _ = &app; // no delayed refresh threads in test binaries
}

pub fn set_chosen(id: Option<String>) {
    *CHOSEN.lock().unwrap_or_else(|p| p.into_inner()) = id;
}

/// State shape: `{"deviceId": str|null, "connected": bool}` (spec 5.3).
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub device_id: Option<String>,
    pub connected: bool,
}

/// One Bluetooth audio device as the picker shows it (spec 6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BtDevice {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub battery: Option<u8>,
}

pub struct Bluetooth;

impl Switch for Bluetooth {
    fn id(&self) -> SwitchId {
        SwitchId::Bluetooth
    }

    fn experimental(&self) -> bool {
        true // undocumented IKsControl property (D-014)
    }

    fn availability(&self, _os: &OsInfo) -> Availability {
        Availability::Available // the probe decides (spec 7.6)
    }

    fn persistent(&self) -> bool {
        true // the connection outlives the process
    }

    fn read(&self) -> Result<serde_json::Value, SwitchError> {
        let _guard = ComGuard::new()?;
        let chosen = chosen();
        let Some(device_id) = chosen else {
            return Ok(serde_json::json!(State {
                device_id: None,
                connected: false,
            }));
        };
        let connected = audio_containers()?
            .iter()
            .any(|c| c.id == device_id && c.connected);
        Ok(serde_json::json!(State {
            device_id: Some(device_id),
            connected,
        }))
    }

    fn is_on(&self, value: &serde_json::Value) -> bool {
        parse_state::<State>(value, "bluetooth")
            .map(|s| s.connected)
            .unwrap_or(false)
    }

    fn target_on(&self, _before: &serde_json::Value, _ctx: &Ctx) -> serde_json::Value {
        serde_json::json!(State {
            device_id: chosen(),
            connected: true,
        })
    }

    fn target_off(&self, _current: &serde_json::Value, _ctx: &Ctx) -> Option<serde_json::Value> {
        Some(serde_json::json!(State {
            device_id: chosen(),
            connected: false,
        }))
    }

    fn apply(&self, value: &serde_json::Value) -> Result<(), SwitchError> {
        let target = parse_state::<State>(value, "bluetooth")?;
        let Some(device_id) = target.device_id.as_deref() else {
            return Err(SwitchError::NotAvailable("Choose a device".into()));
        };
        let _guard = ComGuard::new()?;
        let containers = audio_containers()?;
        let Some(container) = containers.iter().find(|c| c.id == device_id) else {
            return Err(SwitchError::NotAvailable(
                "Bluetooth device not found".into(),
            ));
        };
        if container.ks.is_empty() {
            return Err(SwitchError::NotAvailable(
                "Bluetooth device has no audio endpoint".into(),
            ));
        }
        set_connection(container, target.connected)
    }

    /// Restore addresses the JOURNALED device, not whatever is chosen now —
    /// changing the picker mid-session must not strand the device the
    /// session connected (same rule as microphone's endpointId).
    fn restore(&self, e: &JournalEntry) -> Result<RestoreOutcome, SwitchError> {
        let before = parse_state::<State>(&e.before, "bluetooth")?;
        let after = parse_state::<State>(&e.after, "bluetooth")?;
        let Some(device_id) = before.device_id.as_deref() else {
            return Ok(RestoreOutcome::Gone);
        };
        let _guard = ComGuard::new()?;
        let containers = audio_containers()?;
        let Some(container) = containers.iter().find(|c| c.id == device_id) else {
            return Ok(RestoreOutcome::Gone); // device no longer known
        };
        if container.connected != after.connected {
            return Ok(RestoreOutcome::UserChanged);
        }
        if !after.connected {
            return Ok(RestoreOutcome::Restored); // never connected by us
        }
        set_connection(container, false)?;
        Ok(RestoreOutcome::Restored)
    }

    fn probe(&self) -> Result<(), String> {
        // Natural drop order (guard first => CoUninitialize last) releases
        // the enumerator and collection while the apartment is up.
        let _guard = ComGuard::new().map_err(|e| e.to_string())?;
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .map_err(|e| e.to_string())?;
        unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE(DEVICE_STATEMASK_ALL)) }
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Fires the one-shot reconnect/disconnect property on every IKsControl of
/// the container. Any success counts — one endpoint accepting the property
/// is enough to start the transition (ToothTray prior art).
fn set_connection(container: &Container, connect: bool) -> Result<(), SwitchError> {
    let property = if connect {
        KSPROPERTY_ONESHOT_RECONNECT
    } else {
        KSPROPERTY_ONESHOT_DISCONNECT
    };
    let mut any_ok = false;
    let mut first_error = None;
    for ks in &container.ks {
        let property_id = KSIDENTIFIER {
            Anonymous: windows::Win32::Media::KernelStreaming::KSIDENTIFIER_0 {
                Anonymous: windows::Win32::Media::KernelStreaming::KSIDENTIFIER_0_0 {
                    Set: KSPROPSETID_BtAudio,
                    Id: property.0 as u32,
                    Flags: KSPROPERTY_TYPE_GET,
                },
            },
        };
        let mut returned = 0u32;
        match unsafe {
            ks.KsProperty(
                &property_id,
                std::mem::size_of::<KSIDENTIFIER>() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
            )
        } {
            Ok(()) => any_ok = true,
            Err(e) => {
                log::warn!("bluetooth: KsProperty failed on one endpoint: {e}");
                first_error = first_error.or(Some(e.code().0));
            }
        }
    }
    if !any_ok {
        return Err(SwitchError::Os {
            api: "IKsControl::KsProperty",
            code: first_error.unwrap_or(E_INVALIDARG.0),
        });
    }
    // Connecting takes seconds: refresh the rows (and the UI) once the
    // link has had time to come up (task 4.10).
    #[cfg(not(test))]
    if let Some(app) = APP.get() {
        schedule_delayed_refresh(app.clone());
    }
    Ok(())
}

fn chosen() -> Option<String> {
    CHOSEN.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Canonical container-id string, braces included ("{XXXXXXXX-…}"): the
/// picker stores this in settings and both walks produce it (GUID's Debug
/// alone prints without braces).
fn guid_string(g: &GUID) -> String {
    format!("{{{g:?}}}")
}

/// Connecting settles after seconds (task 4.10): refresh rows at +3 s and
/// +8 s. One short-lived thread per apply; applies are rare.
#[cfg(not(test))]
fn schedule_delayed_refresh(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(3));
        crate::automation::refresh_switches(&app);
        std::thread::sleep(std::time::Duration::from_secs(5));
        crate::automation::refresh_switches(&app);
    });
}

/// One Bluetooth audio container (a device): its id (the GUID string the
/// picker stores), whether any endpoint is ACTIVE, a display-name fallback
/// from the endpoints, and the IKsControl handles apply talks to.
struct Container {
    id: String,
    connected: bool,
    endpoint_name: Option<String>,
    ks: Vec<IKsControl>,
}

/// Every render endpoint, grouped by Bluetooth container. Not-present
/// endpoints (OpenPropertyStore fails with 0xE000020B) are skipped, so an
/// unpaired device disappears and an unplugged one (state 8) stays
/// reachable for reconnect (probe).
fn audio_containers() -> Result<Vec<Container>, SwitchError> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(os("CoCreateInstance"))?;
        let collection: IMMDeviceCollection = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE(DEVICE_STATEMASK_ALL))
            .map_err(os("EnumAudioEndpoints"))?;
        let count = collection.GetCount().map_err(os("GetCount"))?;
        let mut containers: Vec<Container> = Vec::new();
        for i in 0..count {
            let Ok(device) = collection.Item(i) else {
                continue;
            };
            let active = device
                .GetState()
                .map(|s| (s.0 & DEVICE_STATE_ACTIVE.0) != 0)
                .unwrap_or(false);
            // Skip endpoints with no property store (not present).
            let Ok(store) = device.OpenPropertyStore(STGM_READ) else {
                continue;
            };
            let Some(container_guid) = prop_guid(&store, &PKEY_Device_ContainerId) else {
                continue;
            };
            let id = guid_string(&container_guid);
            let endpoint_name = prop_string(&store, &PKEY_Device_FriendlyName).ok();
            let ks = ks_control_of(&enumerator, &device);
            match containers.iter_mut().find(|c| c.id == id) {
                Some(c) => {
                    c.connected |= active;
                    if c.endpoint_name.is_none() {
                        c.endpoint_name = endpoint_name;
                    }
                    if let Some(ks) = ks {
                        c.ks.push(ks);
                    }
                }
                None => containers.push(Container {
                    id,
                    connected: active,
                    endpoint_name,
                    ks: ks.into_iter().collect(),
                }),
            }
        }
        // Only containers with an IKsControl are Bluetooth audio devices;
        // without this, every render endpoint's container (speakers, HDMI)
        // would reach the picker (review finding).
        containers.retain(|c| !c.ks.is_empty());
        Ok(containers)
    }
}

/// The endpoint's IKsControl, found by walking its topology to the
/// Bluetooth device it connects to (probe: the walk works while
/// disconnected too). None when this endpoint has no Bluetooth topology.
fn ks_control_of(enumerator: &IMMDeviceEnumerator, device: &IMMDevice) -> Option<IKsControl> {
    unsafe {
        let topology: IDeviceTopology = device.Activate(CLSCTX_ALL, None).ok()?;
        for c in 0..topology.GetConnectorCount().unwrap_or(0) {
            let Ok(connector) = topology.GetConnector(c) else {
                continue;
            };
            let Ok(other) = connector.GetConnectedTo() else {
                continue; // not connected on this pin
            };
            let Ok(part) = other.cast::<IPart>() else {
                continue;
            };
            let Ok(remote_topology) = part.GetTopologyObject() else {
                continue;
            };
            let Ok(remote_id) = remote_topology.GetDeviceId() else {
                continue;
            };
            let Ok(id_text) = remote_id.to_string() else {
                CoTaskMemFree(Some(remote_id.0 as *const _));
                continue;
            };
            CoTaskMemFree(Some(remote_id.0 as *const _));
            if !id_text.to_ascii_lowercase().starts_with(r"{2}.\?\bth") {
                continue;
            }
            let wide: Vec<u16> = id_text.encode_utf16().chain([0]).collect();
            let remote_device = enumerator.GetDevice(PCWSTR(wide.as_ptr())).ok()?;
            return remote_device.Activate::<IKsControl>(CLSCTX_ALL, None).ok();
        }
    }
    None
}

/// (name, battery) per container id from the BTHENUM device nodes: the
/// FriendlyName of a `BTHENUM\DEV_*` node is the clean device name, and the
/// battery BYTE may sit on any node of the container (probe: the AG node,
/// even while disconnected — hence "show only when connected").
fn bthenum_metadata() -> HashMap<String, (Option<String>, Option<u8>)> {
    let mut map: HashMap<String, (Option<String>, Option<u8>)> = HashMap::new();
    unsafe {
        let filter: Vec<u16> = "BTHENUM".encode_utf16().chain([0]).collect();
        let mut len = 0u32;
        if CM_Get_Device_ID_List_SizeW(
            &mut len,
            PCWSTR(filter.as_ptr()),
            CM_GETIDLIST_FILTER_ENUMERATOR,
        ) != CR_SUCCESS
            || len == 0
        {
            return map;
        }
        let mut buffer = vec![0u16; len as usize];
        if CM_Get_Device_ID_ListW(
            PCWSTR(filter.as_ptr()),
            &mut buffer,
            CM_GETIDLIST_FILTER_ENUMERATOR,
        ) != CR_SUCCESS
        {
            return map;
        }
        for id in buffer.split(|&c| c == 0).filter(|s| !s.is_empty()) {
            let id_text = String::from_utf16_lossy(id);
            let wide: Vec<u16> = id.iter().copied().chain([0]).collect();
            let mut instance = 0u32;
            if CM_Locate_DevNodeW(
                &mut instance,
                PCWSTR(wide.as_ptr()),
                CM_LOCATE_DEVNODE_PHANTOM,
            ) != CR_SUCCESS
            {
                continue;
            }
            let container = devprop(instance, &DEVPKEY_Device_ContainerId)
                .filter(|(t, bytes)| *t == DEVPROP_TYPE_GUID && bytes.len() == 16)
                .map(|(_, bytes)| guid_string(&guid_from_bytes(&bytes)))
                .unwrap_or_default();
            if container.is_empty() {
                continue;
            }
            let entry = map.entry(container).or_insert((None, None));
            if id_text.to_ascii_uppercase().starts_with(r"BTHENUM\DEV_")
                && entry.0.is_none()
                && let Some((prop_type, bytes)) = devprop(instance, &DEVPKEY_Device_FriendlyName)
                && prop_type == DEVPROP_TYPE_STRING
            {
                let wide: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .take_while(|&c| c != 0)
                    .collect();
                entry.0 = Some(String::from_utf16_lossy(&wide));
            }
            if entry.1.is_none()
                && let Some((prop_type, bytes)) = devprop(instance, &DEVPKEY_BLUETOOTH_BATTERY)
                && prop_type == DEVPROP_TYPE_BYTE
                && !bytes.is_empty()
            {
                entry.1 = Some(bytes[0]);
            }
        }
    }
    map
}

/// Decodes the cfgmgr32 GUID property bytes. The buffer is the native GUID
/// layout — Data1 u32-LE, Data2 u16-LE, Data3 u16-LE, then the 8 raw
/// Data4 bytes — so the fields are read one by one. (A u128 round-trip
/// scrambles them: `from_u128` takes Data1 from the HIGH bits.)
fn guid_from_bytes(bytes: &[u8]) -> GUID {
    GUID::from_values(
        u32::from_le_bytes(bytes[0..4].try_into().expect("16 bytes checked by caller")),
        u16::from_le_bytes(bytes[4..6].try_into().expect("16 bytes checked by caller")),
        u16::from_le_bytes(bytes[6..8].try_into().expect("16 bytes checked by caller")),
        bytes[8..16].try_into().expect("16 bytes checked by caller"),
    )
}

/// Raw CM_Get_DevNode_PropertyW value (probe helper): (type, bytes).
unsafe fn devprop(instance: u32, key: &DEVPROPKEY) -> Option<(DEVPROPTYPE, Vec<u8>)> {
    unsafe {
        let mut ty = DEVPROPTYPE(0);
        let mut size = 0u32;
        let _ = CM_Get_DevNode_PropertyW(instance, key, &mut ty, None, &mut size, 0);
        if size == 0 {
            return None;
        }
        let mut buffer = vec![0u8; size as usize];
        if CM_Get_DevNode_PropertyW(
            instance,
            key,
            &mut ty,
            Some(buffer.as_mut_ptr()),
            &mut size,
            0,
        ) != CR_SUCCESS
        {
            return None;
        }
        Some((ty, buffer))
    }
}

fn take(p: PWSTR) -> String {
    unsafe {
        let text = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const _));
        text
    }
}

fn prop_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Result<String, SwitchError> {
    unsafe {
        let mut variant = store
            .GetValue(key)
            .map_err(os("IPropertyStore::GetValue"))?;
        let text = PropVariantToStringAlloc(&variant)
            .map(take)
            .map_err(os("PropVariantToStringAlloc"));
        let _ = PropVariantClear(&mut variant);
        text
    }
}

fn prop_guid(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<GUID> {
    unsafe {
        let mut variant = store.GetValue(key).ok()?;
        let guid = PropVariantToGUID(&variant).ok();
        let _ = PropVariantClear(&mut variant);
        guid
    }
}

fn os(api: &'static str) -> impl Fn(windows::core::Error) -> SwitchError {
    move |e| SwitchError::Os {
        api,
        code: e.code().0,
    }
}

/// The device picker's list (spec 6.1 list_bt_devices): every Bluetooth
/// audio container with name and battery ("n/a" -> None when unknown or
/// disconnected).
pub fn list_devices() -> Result<Vec<BtDevice>, SwitchError> {
    let _guard = ComGuard::new()?;
    let containers = audio_containers()?;
    let metadata = bthenum_metadata();
    Ok(containers
        .into_iter()
        .map(|c| {
            let (dev_name, battery) = metadata.get(&c.id).cloned().unwrap_or((None, None));
            BtDevice {
                name: dev_name.or(c.endpoint_name).unwrap_or_else(|| c.id.clone()),
                battery: battery.filter(|_| c.connected), // stale while disconnected
                id: c.id,
                connected: c.connected,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_shape_round_trips() {
        let state = parse_state::<State>(
            &serde_json::json!({ "deviceId": "{ABC}", "connected": true }),
            "t",
        )
        .unwrap();
        assert_eq!(state.device_id.as_deref(), Some("{ABC}"));
        assert!(state.connected);
        // deviceId is an Option, so serde accepts its absence as null
        // (reads before a device is chosen produce exactly that).
        assert!(parse_state::<State>(&serde_json::json!({ "connected": true }), "t").is_ok());
        assert!(parse_state::<State>(&serde_json::json!(true), "t").is_err());
    }

    #[test]
    fn bluetooth_is_experimental_and_its_probe_is_read_only() {
        assert!(Bluetooth.experimental());
        // On CI there is no audio endpoint enumerator story that matters:
        // the probe may fail; it must never change state (create + enum).
        let _ = Bluetooth.probe();
    }

    #[test]
    fn chosen_device_round_trip() {
        set_chosen(Some("{GUID}".into()));
        assert_eq!(chosen().as_deref(), Some("{GUID}"));
        set_chosen(None);
        assert_eq!(chosen(), None);
    }

    #[test]
    fn guid_bytes_decode_in_the_native_layout() {
        // {104EA319-6EE2-4701-BD47-8DDBF425BBE5} as Windows stores it:
        // Data1/Data2/Data3 little-endian, Data4 raw.
        let guid = GUID::from_values(
            0x104EA319,
            0x6EE2,
            0x4701,
            [0xBD, 0x47, 0x8D, 0xDB, 0xF4, 0x25, 0xBB, 0xE5],
        );
        let bytes: [u8; 16] = [
            0x19, 0xA3, 0x4E, 0x10, // Data1 LE
            0xE2, 0x6E, // Data2 LE
            0x01, 0x47, // Data3 LE
            0xBD, 0x47, 0x8D, 0xDB, 0xF4, 0x25, 0xBB, 0xE5,
        ];
        assert_eq!(guid_from_bytes(&bytes), guid);
        assert_eq!(
            guid_string(&guid_from_bytes(&bytes)),
            "{104EA319-6EE2-4701-BD47-8DDBF425BBE5}"
        );
    }

    /// Read-only live probe of the walk. A machine with no Bluetooth audio
    /// returns an empty list; only COM failures error.
    #[test]
    fn list_devices_walks_or_errors_cleanly() {
        match list_devices() {
            Ok(devices) => {
                for d in &devices {
                    assert!(d.id.starts_with('{'), "container ids are GUID strings");
                }
            }
            Err(SwitchError::Os { .. }) => {}
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}
