//! Floss Bluetooth stack.
//!
//! This crate provides the API implementation of the Fluoride/GD Bluetooth
//! stack, independent of any RPC projection.

#[macro_use]
extern crate num_derive;

pub mod battery_manager;
pub mod battery_provider_manager;
pub mod battery_service;
pub mod bluetooth;
pub mod bluetooth_adv;
pub mod bluetooth_gatt;
pub mod bluetooth_media;
pub mod callbacks;
pub mod socket_manager;
pub mod suspend;
pub mod uuid;

use log::debug;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::channel;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::battery_manager::BatteryManager;
use crate::battery_service::{BatteryService, GattBatteryCallbacks};
use crate::bluetooth::{
    dispatch_base_callbacks, dispatch_hid_host_callbacks, dispatch_sdp_callbacks, Bluetooth,
    IBluetooth,
};
use crate::bluetooth_gatt::{
    dispatch_gatt_client_callbacks, dispatch_le_adv_callbacks, dispatch_le_scanner_callbacks,
    dispatch_le_scanner_inband_callbacks, BluetoothGatt,
};
use crate::bluetooth_media::{BluetoothMedia, MediaActions};
use crate::socket_manager::{BluetoothSocketManager, SocketActions};
use crate::suspend::Suspend;
use bt_topshim::{
    btif::BaseCallbacks,
    profiles::{
        a2dp::A2dpCallbacks, avrcp::AvrcpCallbacks, gatt::GattAdvCallbacks,
        gatt::GattAdvInbandCallbacks, gatt::GattClientCallbacks, gatt::GattScannerCallbacks,
        gatt::GattScannerInbandCallbacks, gatt::GattServerCallbacks, hfp::HfpCallbacks,
        hid_host::HHCallbacks, sdp::SdpCallbacks,
    },
};

/// Message types that are sent to the stack main dispatch loop.
pub enum Message {
    // Shuts down the stack.
    Shutdown,

    // Callbacks from libbluetooth
    A2dp(A2dpCallbacks),
    Avrcp(AvrcpCallbacks),
    Base(BaseCallbacks),
    GattClient(GattClientCallbacks),
    GattServer(GattServerCallbacks),
    LeScanner(GattScannerCallbacks),
    LeScannerInband(GattScannerInbandCallbacks),
    LeAdvInband(GattAdvInbandCallbacks),
    LeAdv(GattAdvCallbacks),
    HidHost(HHCallbacks),
    Hfp(HfpCallbacks),
    Sdp(SdpCallbacks),

    // Actions within the stack
    Media(MediaActions),
    MediaCallbackDisconnected(u32),

    // Client callback disconnections
    AdapterCallbackDisconnected(u32),
    ConnectionCallbackDisconnected(u32),

    // Update list of found devices and remove old instances.
    DeviceFreshnessCheck,

    // Suspend related
    SuspendCallbackRegistered(u32),
    SuspendCallbackDisconnected(u32),
    SuspendReady(u32),

    // Scanner related
    ScannerCallbackDisconnected(u32),

    // Advertising related
    AdvertiserCallbackDisconnected(u32),

    SocketManagerActions(SocketActions),
    SocketManagerCallbackDisconnected(u32),

    // Battery related
    BatteryServiceCallbackDisconnected(u32),
    BatteryServiceCallbacks(GattBatteryCallbacks),
    BatteryManagerCallbackDisconnected(u32),

    GattClientCallbackDisconnected(u32),
}

/// Represents suspend mode of a module.
///
/// Being in suspend mode means that the module pauses some activities if required for suspend and
/// some subsequent API calls will be blocked with a retryable error.
#[derive(FromPrimitive, ToPrimitive)]
pub enum SuspendMode {
    Normal = 0,
    Suspending = 1,
    Suspended = 2,
    Resuming = 3,
}

/// Umbrella class for the Bluetooth stack.
pub struct Stack {}

impl Stack {
    /// Creates an mpsc channel for passing messages to the main dispatch loop.
    pub fn create_channel() -> (Sender<Message>, Receiver<Message>) {
        channel::<Message>(1)
    }

    /// Runs the main dispatch loop.
    pub async fn dispatch(
        mut rx: Receiver<Message>,
        bluetooth: Arc<Mutex<Box<Bluetooth>>>,
        bluetooth_gatt: Arc<Mutex<Box<BluetoothGatt>>>,
        battery_service: Arc<Mutex<Box<BatteryService>>>,
        battery_manager: Arc<Mutex<Box<BatteryManager>>>,
        bluetooth_media: Arc<Mutex<Box<BluetoothMedia>>>,
        suspend: Arc<Mutex<Box<Suspend>>>,
        bluetooth_socketmgr: Arc<Mutex<Box<BluetoothSocketManager>>>,
    ) {
        loop {
            let m = rx.recv().await;

            if m.is_none() {
                eprintln!("Message dispatch loop quit");
                break;
            }

            match m.unwrap() {
                Message::Shutdown => {
                    bluetooth.lock().unwrap().disable();
                }

                Message::A2dp(a) => {
                    bluetooth_media.lock().unwrap().dispatch_a2dp_callbacks(a);
                }

                Message::Avrcp(av) => {
                    bluetooth_media.lock().unwrap().dispatch_avrcp_callbacks(av);
                }

                Message::Base(b) => {
                    dispatch_base_callbacks(bluetooth.lock().unwrap().as_mut(), b.clone());
                    dispatch_base_callbacks(suspend.lock().unwrap().as_mut(), b);
                }

                Message::GattClient(m) => {
                    dispatch_gatt_client_callbacks(bluetooth_gatt.lock().unwrap().as_mut(), m);
                }

                Message::GattServer(m) => {
                    // TODO(b/193685149): dispatch GATT server callbacks.
                    debug!("Unhandled Message::GattServer: {:?}", m);
                }

                Message::LeScanner(m) => {
                    dispatch_le_scanner_callbacks(bluetooth_gatt.lock().unwrap().as_mut(), m);
                }

                Message::LeScannerInband(m) => {
                    dispatch_le_scanner_inband_callbacks(
                        bluetooth_gatt.lock().unwrap().as_mut(),
                        m,
                    );
                }

                Message::LeAdvInband(m) => {
                    debug!("Received LeAdvInband message: {:?}. This is unexpected!", m);
                }

                Message::LeAdv(m) => {
                    dispatch_le_adv_callbacks(bluetooth_gatt.lock().unwrap().as_mut(), m);
                }

                Message::Hfp(hf) => {
                    bluetooth_media.lock().unwrap().dispatch_hfp_callbacks(hf);
                }

                Message::HidHost(h) => {
                    dispatch_hid_host_callbacks(bluetooth.lock().unwrap().as_mut(), h);
                }

                Message::Sdp(s) => {
                    dispatch_sdp_callbacks(bluetooth.lock().unwrap().as_mut(), s);
                }

                Message::Media(action) => {
                    bluetooth_media.lock().unwrap().dispatch_media_actions(action);
                }

                Message::MediaCallbackDisconnected(cb_id) => {
                    bluetooth_media.lock().unwrap().remove_callback(cb_id);
                }

                Message::AdapterCallbackDisconnected(id) => {
                    bluetooth.lock().unwrap().adapter_callback_disconnected(id);
                }

                Message::ConnectionCallbackDisconnected(id) => {
                    bluetooth.lock().unwrap().connection_callback_disconnected(id);
                }

                Message::DeviceFreshnessCheck => {
                    bluetooth.lock().unwrap().trigger_freshness_check();
                }

                Message::SuspendCallbackRegistered(id) => {
                    suspend.lock().unwrap().callback_registered(id);
                }

                Message::SuspendCallbackDisconnected(id) => {
                    suspend.lock().unwrap().remove_callback(id);
                }

                Message::SuspendReady(suspend_id) => {
                    suspend.lock().unwrap().suspend_ready(suspend_id);
                }

                Message::ScannerCallbackDisconnected(id) => {
                    bluetooth_gatt.lock().unwrap().remove_scanner_callback(id);
                }

                Message::AdvertiserCallbackDisconnected(id) => {
                    bluetooth_gatt.lock().unwrap().remove_adv_callback(id);
                }

                Message::SocketManagerActions(action) => {
                    bluetooth_socketmgr.lock().unwrap().handle_actions(action);
                }
                Message::SocketManagerCallbackDisconnected(id) => {
                    bluetooth_socketmgr.lock().unwrap().remove_callback(id);
                }
                Message::BatteryServiceCallbackDisconnected(id) => {
                    battery_service.lock().unwrap().remove_callback(id);
                }
                Message::BatteryServiceCallbacks(callback) => {
                    battery_service.lock().unwrap().handle_callback(callback);
                }
                Message::BatteryManagerCallbackDisconnected(id) => {
                    battery_manager.lock().unwrap().remove_callback(id);
                }
                Message::GattClientCallbackDisconnected(id) => {
                    bluetooth_gatt.lock().unwrap().remove_client_callback(id);
                }
            }
        }
    }
}

/// Signifies that the object may be a proxy to a remote RPC object.
///
/// An object that implements RPCProxy trait signifies that the object may be a proxy to a remote
/// RPC object. Therefore the object may be disconnected and thus should implement
/// `register_disconnect` to let others observe the disconnection event.
pub trait RPCProxy {
    /// Registers disconnect observer that will be notified when the remote object is disconnected.
    fn register_disconnect(&mut self, _f: Box<dyn Fn(u32) + Send>) -> u32 {
        0
    }

    /// Returns the ID of the object. For example this would be an object path in D-Bus RPC.
    fn get_object_id(&self) -> String {
        String::from("")
    }

    /// Unregisters callback with this id.
    fn unregister(&mut self, _id: u32) -> bool {
        false
    }

    /// Makes this object available for remote call.
    fn export_for_rpc(self: Box<Self>) {}
}
