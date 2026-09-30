//! The `ios-simulator::*` wire surface: typed request/response structs and
//! their registration. Every call takes an optional `tenant` (see
//! `tenant.rs`); `catalog()` lists the surface for the schema tests.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::bridge::{now_ms, WATCH_LEASE_MS};
use crate::input;
use crate::sim::{content_blocks, MediaInfo, Sim, MAX_READ_CHUNK};
use crate::simctl::{Device, Runtime};
use crate::tenant;

// ── shared shapes ───────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TenantInput {
    /// Tenant whose simulators to use. Omit for `default`.
    #[serde(default)]
    pub tenant: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeviceInput {
    #[serde(default)]
    pub tenant: Option<String>,
    /// Simulator UDID from `ios-simulator::devices::list`.
    pub udid: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BootInput {
    #[serde(default)]
    pub tenant: Option<String>,
    pub udid: String,
    /// Show the console's live preview for this boot. Default true; the
    /// Simulators page passes false because it already shows the phone.
    #[serde(default)]
    pub preview: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeviceInfo {
    #[serde(flatten)]
    pub device: Device,
    /// Someone is watching the live view.
    pub streaming: bool,
    pub recording: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeviceOutput {
    pub tenant: String,
    pub device: Device,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Ack {
    pub ok: bool,
}

// ── inputs and outputs ──────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
pub struct TenantsOutput {
    pub tenants: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RuntimesOutput {
    pub runtimes: Vec<Runtime>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DevicesOutput {
    pub tenant: String,
    pub devices: Vec<DeviceInfo>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateInput {
    #[serde(default)]
    pub tenant: Option<String>,
    /// Display name, e.g. `Checkout iPhone`.
    pub name: String,
    /// Device type identifier from `ios-simulator::runtimes`.
    pub device_type: String,
    /// Runtime identifier; omit for the newest one that runs `device_type`.
    #[serde(default)]
    pub runtime: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotInput {
    #[serde(default)]
    pub tenant: Option<String>,
    pub udid: String,
    /// Return a ≤1280px JPEG preview in `content`. Default true; the PNG is
    /// saved either way.
    #[serde(default)]
    pub include_image: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ScreenshotDetails {
    pub media: MediaInfo,
    /// The simulator's name.
    pub device: String,
    /// Screenshot pixels: the coordinate space of touch and gesture.
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ScreenshotOutput {
    /// `[{type:image,…}?, {type:text,…}]`.
    pub content: Value,
    pub details: ScreenshotDetails,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RecordingStartOutput {
    /// Media name the finished recording will have.
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RecordingStopOutput {
    /// False when nothing was recording.
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaInfo>,
    pub duration_ms: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MediaListInput {
    #[serde(default)]
    pub tenant: Option<String>,
    /// Only this simulator's media.
    #[serde(default)]
    pub udid: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MediaListOutput {
    /// Newest first.
    pub media: Vec<MediaInfo>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MediaInput {
    #[serde(default)]
    pub tenant: Option<String>,
    /// Media name from `media::list` (`<udid>/<file>`).
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MediaReadInput {
    #[serde(default)]
    pub tenant: Option<String>,
    pub name: String,
    /// Byte offset to read from. Default 0.
    #[serde(default)]
    pub offset: Option<u64>,
    /// Bytes to read, at most 8 MiB per call. Default 8 MiB.
    #[serde(default)]
    pub length: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MediaReadOutput {
    /// Base64 of the slice.
    pub data: String,
    pub mime: String,
    pub offset: u64,
    pub total_bytes: u64,
    /// True when this slice ends the file.
    pub eof: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MediaDeleteOutput {
    pub ok: bool,
    /// False when there was no such file.
    pub removed: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TouchInput {
    #[serde(default)]
    pub tenant: Option<String>,
    pub udid: String,
    /// `down`, `move` or `up`.
    pub phase: String,
    /// Screenshot pixels (top-left origin).
    pub x: f64,
    pub y: f64,
    /// A second finger for pinch and two-finger drags.
    #[serde(default)]
    pub x2: Option<f64>,
    #[serde(default)]
    pub y2: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GestureInput {
    #[serde(default)]
    pub tenant: Option<String>,
    pub udid: String,
    /// `tap`, `double_tap`, `long_press`, `swipe` or `pinch`.
    pub kind: String,
    /// Where the gesture starts (pinch: its center), in screenshot pixels.
    pub x: f64,
    pub y: f64,
    /// Swipe end point.
    #[serde(default)]
    pub to_x: Option<f64>,
    #[serde(default)]
    pub to_y: Option<f64>,
    /// Pinch: finger distance at the start and the end, in pixels. Growing
    /// zooms in, shrinking zooms out.
    #[serde(default)]
    pub from_distance: Option<f64>,
    #[serde(default)]
    pub to_distance: Option<f64>,
    /// Swipe/pinch/long-press duration. Defaults: 300 / 400 / 800 ms.
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ButtonInput {
    #[serde(default)]
    pub tenant: Option<String>,
    pub udid: String,
    /// `home`, `lock` (side button), `action`, `volume_up` or `volume_down`.
    pub button: String,
    /// `press` (default), `down` or `up`.
    #[serde(default)]
    pub phase: Option<String>,
    /// How long a `press` holds the button. Default 100 ms.
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TypeInput {
    #[serde(default)]
    pub tenant: Option<String>,
    pub udid: String,
    /// Text for the focused field. US-keyboard characters are typed; anything
    /// else is pasted.
    pub text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyInput {
    #[serde(default)]
    pub tenant: Option<String>,
    pub udid: String,
    /// One key or a chord: `["enter"]`, `["cmd", "a"]`, `["shift", "tab"]`.
    pub keys: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AppInput {
    #[serde(default)]
    pub tenant: Option<String>,
    pub udid: String,
    /// e.g. `com.apple.mobilesafari`.
    pub bundle_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenUrlInput {
    #[serde(default)]
    pub tenant: Option<String>,
    pub udid: String,
    /// Any URL the simulator can open: `https://…` or an app scheme.
    pub url: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WatchOutput {
    /// Device pixels: the coordinate space of touch.
    pub device_width: u32,
    pub device_height: u32,
    /// Call again before this lapses to keep the frames coming.
    pub lease_ms: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FrameInput {
    #[serde(default)]
    pub tenant: Option<String>,
    pub udid: String,
    /// The `seq` you already have; the reply omits `frame` while it is current.
    #[serde(default)]
    pub since_seq: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FrameOutput {
    /// Base64 JPEG; absent when nothing newer than `since_seq` exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
    pub width: u32,
    pub height: u32,
    pub device_width: u32,
    pub device_height: u32,
    pub seq: u64,
    pub timestamp: i64,
}

// ── catalog ─────────────────────────────────────────────────────────────

pub struct FunctionSpec {
    pub id: &'static str,
    pub description: &'static str,
    pub request: schemars::schema::RootSchema,
    pub response: schemars::schema::RootSchema,
}

fn spec<Req: JsonSchema, Resp: JsonSchema>(
    id: &'static str,
    description: &'static str,
) -> FunctionSpec {
    let generator = || schemars::r#gen::SchemaSettings::draft07().into_generator();
    FunctionSpec {
        id,
        description,
        request: generator().into_root_schema_for::<Req>(),
        response: generator().into_root_schema_for::<Resp>(),
    }
}

macro_rules! functions {
    ($( $name:ident = $id:literal, $desc:literal, $req:ty => $resp:ty; )*) => {
        $( pub const $name: (&str, &str) = ($id, $desc); )*
        /// The whole wire surface, in registration order.
        pub fn catalog() -> Vec<FunctionSpec> {
            vec![$( spec::<$req, $resp>($id, $desc) ),*]
        }
    };
}

functions! {
    TENANTS = "ios-simulator::tenants::list",
        "Operator: list tenants with simulators or media on this Mac. Never expose to tenants.",
        TenantInput => TenantsOutput;
    RUNTIMES = "ios-simulator::runtimes",
        "List installed simulator runtimes (iOS versions) and the device types each can run; feed them to devices::create.",
        TenantInput => RuntimesOutput;
    DEVICES_LIST = "ios-simulator::devices::list",
        "List the tenant's simulators with their state (Booted, Shutdown, …).",
        TenantInput => DevicesOutput;
    DEVICES_CREATE = "ios-simulator::devices::create",
        "Create a simulator for the tenant from a device type (and optional runtime). It starts shut down; boot it next.",
        CreateInput => DeviceOutput;
    DEVICES_DELETE = "ios-simulator::devices::delete",
        "Delete one of the tenant's simulators with its screenshots and recordings. Shuts it down first.",
        DeviceInput => Ack;
    DEVICES_BOOT = "ios-simulator::devices::boot",
        "Boot a simulator (headless; no Simulator.app window). Refused past the Mac-wide or per-tenant booted caps.",
        BootInput => DeviceOutput;
    DEVICES_SHUTDOWN = "ios-simulator::devices::shutdown",
        "Shut a simulator down, stopping its live view and any recording.",
        DeviceInput => DeviceOutput;
    DEVICES_ERASE = "ios-simulator::devices::erase",
        "Erase a shut-down simulator's content and settings (factory reset).",
        DeviceInput => DeviceOutput;
    OPEN = "ios-simulator::open",
        "Open a simulator for the user: a live iPhone floating over the console, booting it first if it is shut down. Use it (not console::workspace::open) whenever the user asks to open, show or watch a simulator; driving one with gesture, type, button, apps or screenshot shows it too.",
        DeviceInput => DeviceOutput;
    SCREENSHOT = "ios-simulator::screenshot",
        "Take a screenshot of a booted simulator. Saves the PNG to the tenant's media and returns a preview; its pixels are the coordinate space of touch and gesture.",
        ScreenshotInput => ScreenshotOutput;
    RECORDING_START = "ios-simulator::recording::start",
        "Start recording a booted simulator's screen to an H.264 QuickTime movie (.mov) in the tenant's media.",
        DeviceInput => RecordingStartOutput;
    RECORDING_STOP = "ios-simulator::recording::stop",
        "Stop a recording and finalize the file. Succeeds with ok=false when nothing was recording.",
        DeviceInput => RecordingStopOutput;
    MEDIA_LIST = "ios-simulator::media::list",
        "List the tenant's saved screenshots and recordings, newest first.",
        MediaListInput => MediaListOutput;
    MEDIA_READ = "ios-simulator::media::read",
        "Read a saved screenshot or recording as base64, up to 8 MiB per call; page with offset until eof.",
        MediaReadInput => MediaReadOutput;
    MEDIA_DELETE = "ios-simulator::media::delete",
        "Delete a saved screenshot or recording.",
        MediaInput => MediaDeleteOutput;
    GESTURE = "ios-simulator::gesture",
        "Perform a touch gesture on a booted simulator: tap, double_tap, long_press, swipe or pinch, in screenshot pixels (top-left origin). Swipe up from the bottom edge to go home.",
        GestureInput => Ack;
    BUTTON = "ios-simulator::button",
        "Press a hardware button: home, lock (the side button; hold for Siri), action, volume_up or volume_down.",
        ButtonInput => Ack;
    TYPE = "ios-simulator::type",
        "Type text into the focused field of a booted simulator.",
        TypeInput => Ack;
    KEY = "ios-simulator::key",
        "Press a key or chord on the simulator's hardware keyboard: enter, backspace, tab, arrows, or [\"cmd\", \"a\"].",
        KeyInput => Ack;
    APP_LAUNCH = "ios-simulator::apps::launch",
        "Launch an installed app by bundle id.",
        AppInput => Ack;
    APP_TERMINATE = "ios-simulator::apps::terminate",
        "Terminate a running app by bundle id.",
        AppInput => Ack;
    OPEN_URL = "ios-simulator::open-url",
        "Open a URL on the simulator (Safari, or the app that owns the scheme).",
        OpenUrlInput => Ack;
    TOUCH = "ios-simulator::touch",
        "Internal: one raw touch phase (down/move/up, optional second finger) for the live view. Agents use ios-simulator::gesture.",
        TouchInput => Ack;
    WATCH = "ios-simulator::watch",
        "Internal: start or renew the live view of a simulator for 15s; frames arrive as ios-simulator::frame-event triggers bound to its udid. Console plumbing; agents use ios-simulator::screenshot.",
        DeviceInput => WatchOutput;
    FRAME = "ios-simulator::frame",
        "Internal: newest live-view frame of a watched simulator, or nothing when since_seq is current. Not an agent function.",
        FrameInput => FrameOutput;
}

// ── registration ────────────────────────────────────────────────────────

fn handler_err(e: String) -> Error {
    Error::Handler(e)
}

/// Register one async function whose body gets the shared `Sim`.
fn register<Req, Resp, F, Fut>(iii: &IIIClient, sim: &Arc<Sim>, (id, desc): (&str, &str), body: F)
where
    Req: serde::de::DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Sim>, Req) -> Fut + Send + Sync + Copy + 'static,
    Fut: std::future::Future<Output = Result<Resp, String>> + Send + 'static,
{
    let sim = sim.clone();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |req: Req| {
            let sim = sim.clone();
            async move { body(sim, req).await.map_err(handler_err) }
        })
        .description(desc),
    );
}

async fn device_info(sim: &Sim, tenant: &str, device: Device) -> DeviceInfo {
    DeviceInfo {
        streaming: sim
            .bridges
            .find(tenant, &device.udid)
            .is_some_and(|b| b.is_streaming()),
        recording: sim.is_recording(tenant, &device.udid).await,
        device,
    }
}

async fn run_app(
    sim: &Sim,
    req: &AppInput,
    (id, _): (&str, &str),
    verb: &str,
) -> Result<Ack, String> {
    let t = sim.tenant(req.tenant.as_deref())?;
    sim.device(&t, &req.udid).await?;
    t.set.run(&[verb, &req.udid, &req.bundle_id]).await?;
    sim.used(&t.name, &req.udid, id);
    Ok(Ack { ok: true })
}

pub fn register_all(iii: &Arc<IIIClient>, sim: &Arc<Sim>) {
    register(iii, sim, TENANTS, |sim, _req: TenantInput| async move {
        Ok(TenantsOutput {
            tenants: tenant::known(&sim.config.load()),
        })
    });
    register(iii, sim, RUNTIMES, |sim, req: TenantInput| async move {
        let t = sim.tenant(req.tenant.as_deref())?;
        Ok(RuntimesOutput {
            runtimes: sim.runtimes(&t).await?,
        })
    });
    register(iii, sim, DEVICES_LIST, |sim, req: TenantInput| async move {
        let t = sim.tenant(req.tenant.as_deref())?;
        let mut devices = Vec::new();
        for d in sim.devices(&t).await? {
            devices.push(device_info(&sim, &t.name, d).await);
        }
        Ok(DevicesOutput {
            tenant: t.name,
            devices,
        })
    });
    register(
        iii,
        sim,
        DEVICES_CREATE,
        |sim, req: CreateInput| async move {
            let t = sim.tenant(req.tenant.as_deref())?;
            let device = sim
                .create(&t, &req.name, &req.device_type, req.runtime.as_deref())
                .await?;
            Ok(DeviceOutput {
                tenant: t.name,
                device,
            })
        },
    );
    register(
        iii,
        sim,
        DEVICES_DELETE,
        |sim, req: DeviceInput| async move {
            let t = sim.tenant(req.tenant.as_deref())?;
            sim.delete(&t, &req.udid).await?;
            Ok(Ack { ok: true })
        },
    );
    register(iii, sim, DEVICES_BOOT, |sim, req: BootInput| async move {
        let t = sim.tenant(req.tenant.as_deref())?;
        let device = sim.boot(&t, &req.udid, req.preview.unwrap_or(true)).await?;
        Ok(DeviceOutput {
            tenant: t.name,
            device,
        })
    });
    register(iii, sim, OPEN, |sim, req: DeviceInput| async move {
        let t = sim.tenant(req.tenant.as_deref())?;
        let device = sim.boot(&t, &req.udid, true).await?;
        sim.used(&t.name, &req.udid, OPEN.0);
        Ok(DeviceOutput {
            tenant: t.name,
            device,
        })
    });
    register(
        iii,
        sim,
        DEVICES_SHUTDOWN,
        |sim, req: DeviceInput| async move {
            let t = sim.tenant(req.tenant.as_deref())?;
            let device = sim.shutdown(&t, &req.udid).await?;
            Ok(DeviceOutput {
                tenant: t.name,
                device,
            })
        },
    );
    register(
        iii,
        sim,
        DEVICES_ERASE,
        |sim, req: DeviceInput| async move {
            let t = sim.tenant(req.tenant.as_deref())?;
            let device = sim.erase(&t, &req.udid).await?;
            Ok(DeviceOutput {
                tenant: t.name,
                device,
            })
        },
    );
    register(
        iii,
        sim,
        SCREENSHOT,
        |sim, req: ScreenshotInput| async move {
            let t = sim.tenant(req.tenant.as_deref())?;
            let shot = sim
                .screenshot(&t, &req.udid, req.include_image.unwrap_or(true))
                .await?;
            sim.used(&t.name, &req.udid, SCREENSHOT.0);
            let (width, height) = (shot.width, shot.height);
            let mut text = format!(
                "Screenshot of {} ({}) saved as {}. Touch coordinates are {width}x{height} px",
                shot.device, req.udid, shot.media.name
            );
            let longest = width.max(height).max(1);
            if let Some(p) = shot.preview.as_ref().filter(|p| p.edge < longest) {
                let scale = f64::from(longest) / f64::from(p.edge);
                text += &format!(
                    "; this preview is {}x{}, so multiply what you read off it by {scale:.3}",
                    (f64::from(width) / scale).round(),
                    (f64::from(height) / scale).round(),
                );
            }
            text.push('.');
            Ok(ScreenshotOutput {
                content: content_blocks(shot.preview.map(|p| p.data), text),
                details: ScreenshotDetails {
                    media: shot.media,
                    device: shot.device,
                    width,
                    height,
                },
            })
        },
    );
    register(
        iii,
        sim,
        RECORDING_START,
        |sim, req: DeviceInput| async move {
            let t = sim.tenant(req.tenant.as_deref())?;
            let name = sim.recording_start(&t, &req.udid).await?;
            sim.used(&t.name, &req.udid, RECORDING_START.0);
            Ok(RecordingStartOutput { name })
        },
    );
    register(
        iii,
        sim,
        RECORDING_STOP,
        |sim, req: DeviceInput| async move {
            let t = sim.tenant(req.tenant.as_deref())?;
            Ok(match sim.recording_stop(&t, &req.udid).await? {
                Some((media, duration_ms)) => RecordingStopOutput {
                    ok: true,
                    media: Some(media),
                    duration_ms,
                },
                None => RecordingStopOutput {
                    ok: false,
                    media: None,
                    duration_ms: 0,
                },
            })
        },
    );
    register(
        iii,
        sim,
        MEDIA_LIST,
        |sim, req: MediaListInput| async move {
            let t = sim.tenant(req.tenant.as_deref())?;
            Ok(MediaListOutput {
                media: sim.media_list(&t, req.udid.as_deref()).await?,
            })
        },
    );
    register(
        iii,
        sim,
        MEDIA_READ,
        |sim, req: MediaReadInput| async move {
            let t = sim.tenant(req.tenant.as_deref())?;
            let offset = req.offset.unwrap_or(0);
            let length = req.length.unwrap_or(MAX_READ_CHUNK);
            let (data, read, total) = sim.media_read(&t, &req.name, offset, length).await?;
            Ok(MediaReadOutput {
                mime: if req.name.ends_with(".png") {
                    "image/png".into()
                } else {
                    "video/quicktime".into()
                },
                eof: offset + read >= total,
                data,
                offset,
                total_bytes: total,
            })
        },
    );
    register(iii, sim, MEDIA_DELETE, |sim, req: MediaInput| async move {
        let t = sim.tenant(req.tenant.as_deref())?;
        Ok(MediaDeleteOutput {
            ok: true,
            removed: sim.media_delete(&t, &req.name).await?,
        })
    });
    register(iii, sim, GESTURE, |sim, req: GestureInput| async move {
        let t = sim.tenant(req.tenant.as_deref())?;
        let bridge = sim.bridge(&t, &req.udid).await?;
        sim.used(&t.name, &req.udid, GESTURE.0);
        let at = (req.x, req.y);
        let steps = match req.kind.as_str() {
            "tap" => input::tap(req.x, req.y, 60),
            "double_tap" => {
                let mut steps = input::tap(req.x, req.y, 50);
                steps.last_mut().expect("tap has steps").1 = 80;
                steps.extend(input::tap(req.x, req.y, 50));
                steps
            }
            "long_press" => input::tap(req.x, req.y, req.duration_ms.unwrap_or(800)),
            "swipe" => {
                let (Some(to_x), Some(to_y)) = (req.to_x, req.to_y) else {
                    return Err("swipe needs to_x and to_y".into());
                };
                // Starting on the bottom edge is the home-indicator swipe.
                input::swipe(at, (to_x, to_y), req.duration_ms.unwrap_or(300))
            }
            "pinch" => {
                let (Some(from), Some(to)) = (req.from_distance, req.to_distance) else {
                    return Err("pinch needs from_distance and to_distance".into());
                };
                input::pinch(at, from, to, req.duration_ms.unwrap_or(400))
            }
            other => return Err(format!("unknown gesture '{other}'")),
        };
        bridge.play(steps).await?;
        Ok(Ack { ok: true })
    });
    register(iii, sim, BUTTON, |sim, req: ButtonInput| async move {
        let t = sim.tenant(req.tenant.as_deref())?;
        let bridge = sim.bridge(&t, &req.udid).await?;
        sim.used(&t.name, &req.udid, BUTTON.0);
        let cmd = |phase: &str| json!({ "op": "button", "button": req.button, "phase": phase });
        match req.phase.as_deref().unwrap_or("press") {
            "press" => {
                bridge
                    .play(vec![
                        (cmd("down"), req.duration_ms.unwrap_or(100).min(10_000)),
                        (cmd("up"), 0),
                    ])
                    .await?
            }
            phase @ ("down" | "up") => bridge.call(cmd(phase)).await?,
            other => return Err(format!("unknown phase '{other}'")),
        }
        Ok(Ack { ok: true })
    });
    register(iii, sim, TYPE, |sim, req: TypeInput| async move {
        let t = sim.tenant(req.tenant.as_deref())?;
        sim.type_text(&t, &req.udid, &req.text).await?;
        sim.used(&t.name, &req.udid, TYPE.0);
        Ok(Ack { ok: true })
    });
    register(iii, sim, KEY, |sim, req: KeyInput| async move {
        let t = sim.tenant(req.tenant.as_deref())?;
        let bridge = sim.bridge(&t, &req.udid).await?;
        sim.used(&t.name, &req.udid, KEY.0);
        let cmds = input::chord_commands(&req.keys)?;
        bridge
            .play(cmds.into_iter().map(|c| (c, 0)).collect())
            .await?;
        Ok(Ack { ok: true })
    });
    register(iii, sim, APP_LAUNCH, |sim, req: AppInput| async move {
        run_app(&sim, &req, APP_LAUNCH, "launch").await
    });
    register(iii, sim, APP_TERMINATE, |sim, req: AppInput| async move {
        run_app(&sim, &req, APP_TERMINATE, "terminate").await
    });
    register(iii, sim, OPEN_URL, |sim, req: OpenUrlInput| async move {
        let t = sim.tenant(req.tenant.as_deref())?;
        sim.device(&t, &req.udid).await?;
        t.set.run(&["openurl", &req.udid, &req.url]).await?;
        sim.used(&t.name, &req.udid, OPEN_URL.0);
        Ok(Ack { ok: true })
    });
    register(iii, sim, TOUCH, |sim, req: TouchInput| async move {
        let t = sim.tenant(req.tenant.as_deref())?;
        let bridge = sim.bridge(&t, &req.udid).await?;
        if !matches!(req.phase.as_str(), "down" | "move" | "up") {
            return Err("phase must be down, move or up".into());
        }
        let mut cmd = json!({ "op": "touch", "phase": req.phase, "x": req.x, "y": req.y });
        if let (Some(x2), Some(y2)) = (req.x2, req.y2) {
            cmd["x2"] = json!(x2);
            cmd["y2"] = json!(y2);
        }
        // The live view awaits each call before sending the next, so plain
        // ordered writes are enough; waiting for the HID ack would halve
        // the drag rate.
        bridge.send(cmd).await?;
        Ok(Ack { ok: true })
    });
    register(iii, sim, WATCH, |sim, req: DeviceInput| async move {
        let t = sim.tenant(req.tenant.as_deref())?;
        let bridge = sim.bridge(&t, &req.udid).await?;
        bridge.watch(&sim.config.load()).await?;
        let (device_width, device_height) = bridge.size();
        Ok(WatchOutput {
            device_width,
            device_height,
            lease_ms: WATCH_LEASE_MS,
        })
    });
    register(iii, sim, FRAME, |sim, req: FrameInput| async move {
        let t = sim.tenant(req.tenant.as_deref())?;
        let bridge = sim
            .bridges
            .find(&t.name, &req.udid)
            .ok_or("not watched; call ios-simulator::watch first")?;
        let (device_width, device_height) = bridge.size();
        let latest = bridge.latest();
        let fresh = latest
            .as_ref()
            .filter(|f| req.since_seq.is_none_or(|s| f.seq > s));
        Ok(FrameOutput {
            frame: fresh.map(|f| f.data.clone()),
            width: latest.as_ref().map_or(0, |f| f.width),
            height: latest.as_ref().map_or(0, |f| f.height),
            device_width,
            device_height,
            seq: latest.as_ref().map_or(0, |f| f.seq),
            timestamp: latest.as_ref().map_or(now_ms(), |f| f.timestamp),
        })
    });
}
