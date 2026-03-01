use super::*;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NetworkNeighbor {
    pub(crate) ip: String,
    pub(crate) mac: Option<String>,
    pub(crate) interface: Option<String>,
    pub(crate) state: Option<String>,
    pub(crate) is_helios_candidate: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiscoveredDevice {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) status: String,
    pub(crate) connection_chips: Vec<String>,
    pub(crate) ip_address: Option<String>,
    pub(crate) mac_address: Option<String>,
    pub(crate) interface_name: Option<String>,
    pub(crate) usb_location: Option<String>,
    pub(crate) vendor_product: Option<String>,
    pub(crate) runtime_product: Option<String>,
    pub(crate) firmware_version: Option<String>,
    pub(crate) os_version: Option<String>,
    pub(crate) telemetry_summary: Option<String>,
    pub(crate) detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceDiscoverySnapshot {
    pub(crate) generated_at_epoch_ms: u64,
    pub(crate) devices: Vec<DiscoveredDevice>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceDiscoveryProgressEvent {
    pub(crate) generated_at_epoch_ms: u64,
    pub(crate) devices: Vec<DiscoveredDevice>,
    pub(crate) warnings: Vec<String>,
    pub(crate) in_progress: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionStatusSnapshot {
    pub(crate) connected: bool,
    pub(crate) route: String,
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) target_ip: Option<String>,
    pub(crate) interface_name: Option<String>,
    pub(crate) generated_at_epoch_ms: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct RawNeighborEntry {
    pub(crate) ip: String,
    pub(crate) mac: Option<String>,
    pub(crate) interface: Option<String>,
    pub(crate) state: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct IpDeviceCandidate {
    pub(crate) ip: String,
    pub(crate) mac: Option<String>,
    pub(crate) interface: String,
    pub(crate) state: Option<String>,
    pub(crate) is_usb_link: bool,
    pub(crate) usb_identity: Option<UsbIdentity>,
}

#[derive(Debug, Clone)]
pub(crate) struct UsbIdentity {
    pub(crate) vendor_id: String,
    pub(crate) product_id: String,
    pub(crate) manufacturer: Option<String>,
    pub(crate) product: Option<String>,
    pub(crate) serial: Option<String>,
}
