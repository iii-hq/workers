// simbridge — one process per booted simulator the ios-simulator worker is
// driving. It talks to CoreSimulator/SimulatorKit (Xcode's private
// frameworks, the same ones Simulator.app uses) to:
//
// - stream the device framebuffer as JPEG frames (IOSurface seed polling:
//   an unchanged screen costs one seed read per tick, no encode);
// - inject multi-touch, hardware-button and keyboard HID events through
//   SimDeviceLegacyHIDClient with messages built by SimulatorKit's own
//   IndigoHIDMessageFor* functions.
//
// The worker compiles this file on the host with `xcrun swiftc` (Xcode is a
// prerequisite for simulators anyway) and speaks JSON lines to it:
//
//   stdin   {"id":1,"op":"touch","phase":"down|move|up","x":px,"y":px,"x2":px,"y2":px}
//           {"id":2,"op":"button","button":"home|lock|action|volume_up|volume_down","phase":"down|up"}
//           {"id":3,"op":"key","usage":4,"phase":"down|up"}
//           {"op":"frame"}          force the current frame out
//           {"op":"stream","fps":30,"max_dim":1000,"quality":0.7}   fps 0 pauses (input only)
//   stdout  {"type":"ready","width":W,"height":H}
//           {"type":"frame","seq":N,"width":w,"height":h,"device_width":W,"device_height":H,"data":"<jpeg b64>"}
//           {"type":"done","id":1,"error":"..."?}
//           {"type":"error","message":"..."}   fatal, then exit 1
//
// Coordinates are device framebuffer pixels (the screenshot's pixel space).

import CoreImage
import Foundation
import IOSurface
import ObjectiveC

setvbuf(stdout, nil, _IOFBF, 1 << 20)
let out = DispatchQueue(label: "simbridge.out")

func emit(_ obj: [String: Any]) {
  guard let data = try? JSONSerialization.data(withJSONObject: obj) else { return }
  out.sync {
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data([0x0a]))
  }
}

func fail(_ message: String) -> Never {
  emit(["type": "error", "message": message])
  exit(1)
}

// ── arguments ───────────────────────────────────────────────────────────

var udid = ""
var setPath: String?
var developerDir = ProcessInfo.processInfo.environment["DEVELOPER_DIR"] ?? ""
var fps = 30.0
var maxDim = 1000.0
var quality = 0.7
var args = CommandLine.arguments.dropFirst().makeIterator()
while let flag = args.next() {
  let value = args.next() ?? ""
  switch flag {
  case "--udid": udid = value
  case "--set": setPath = value.isEmpty ? nil : value
  case "--developer-dir": developerDir = value
  case "--fps": fps = Double(value) ?? fps
  case "--max-dim": maxDim = Double(value) ?? maxDim
  case "--quality": quality = Double(value) ?? quality
  default: fail("unknown flag \(flag)")
  }
}
if udid.isEmpty { fail("--udid is required") }
if developerDir.isEmpty { fail("--developer-dir is required (xcode-select -p)") }

// ── private frameworks ──────────────────────────────────────────────────

guard dlopen("/Library/Developer/PrivateFrameworks/CoreSimulator.framework/CoreSimulator", RTLD_NOW) != nil
else { fail("CoreSimulator.framework not found; install Xcode") }
guard let simKit = dlopen(developerDir + "/Library/PrivateFrameworks/SimulatorKit.framework/SimulatorKit", RTLD_NOW)
else { fail("SimulatorKit.framework not found under \(developerDir)") }

func sym<T>(_ name: String, _: T.Type) -> T {
  guard let p = dlsym(simKit, name) else { fail("SimulatorKit lacks \(name); unsupported Xcode") }
  return unsafeBitCast(p, to: T.self)
}

func call(_ obj: AnyObject, _ selector: String) -> AnyObject? {
  let sel = NSSelectorFromString(selector)
  guard obj.responds(to: sel) else { return nil }
  return obj.perform(sel)?.takeUnretainedValue()
}

// (point0, point1?, target, NSEventType, edge, size) — divides the points by
// `size`, so passing ratios with size 1×1 keeps us resolution independent.
typealias MouseMessageFn = @convention(c) (
  UnsafeMutablePointer<CGPoint>?, UnsafeMutablePointer<CGPoint>?, UInt32, UInt64, UInt32, CGSize
) -> UnsafeMutableRawPointer?
typealias ButtonMessageFn = @convention(c) (Int32, Int32, Int32) -> UnsafeMutableRawPointer?
typealias KeyMessageFn = @convention(c) (UInt32, UInt32) -> UnsafeMutableRawPointer?
// (target, usage page, usage, direction)
typealias HIDMessageFn = @convention(c) (UInt32, UInt32, UInt32, UInt32) -> UnsafeMutableRawPointer?
let mouseMessage = sym("IndigoHIDMessageForMouseNSEvent", MouseMessageFn.self)
let buttonMessage = sym("IndigoHIDMessageForButton", ButtonMessageFn.self)
let keyMessage = sym("IndigoHIDMessageForKeyboardArbitrary", KeyMessageFn.self)
let hidMessage = sym("IndigoHIDMessageForHIDArbitrary", HIDMessageFn.self)

guard let contextClass: AnyObject = NSClassFromString("SimServiceContext") else { fail("SimServiceContext missing") }
guard
  let context = contextClass.perform(
    NSSelectorFromString("sharedServiceContextForDeveloperDir:error:"), with: developerDir, with: nil
  )?.takeUnretainedValue()
else { fail("no CoreSimulator service context") }

let deviceSet: AnyObject? =
  if let setPath {
    context.perform(NSSelectorFromString("deviceSetWithPath:error:"), with: setPath, with: nil)?
      .takeUnretainedValue()
  } else {
    context.perform(NSSelectorFromString("defaultDeviceSetWithError:"), with: nil)?.takeUnretainedValue()
  }
guard let deviceSet, let byUDID = call(deviceSet, "devicesByUDID") as? NSDictionary,
  let uuid = NSUUID(uuidString: udid), let device = byUDID[uuid] as AnyObject?
else { fail("device \(udid) not found in \(setPath ?? "the default device set")") }

// The main display: the one IO port whose descriptor renders an IOSurface.
guard let deviceIO = call(device, "io") else { fail("device has no IO; is it booted?") }

func surfaceOf(_ display: AnyObject) -> IOSurface? {
  // Masked: rounded corners and the sensor housing come out black, which is
  // exactly what the page's black bezel expects.
  (call(display, "maskedFramebufferSurface") ?? call(display, "framebufferSurface")) as? IOSurface
}

// The main display is the display port that actually carries a surface (an
// idle external/CarPlay port answers nil). A device that just booted may not
// have one yet: wait for it.
var screen: AnyObject?
for _ in 0..<120 {
  for port in (call(deviceIO, "ioPorts") as? [AnyObject]) ?? [] {
    if let descriptor = call(port, "descriptor"),
      descriptor.responds(to: NSSelectorFromString("framebufferSurface")), surfaceOf(descriptor) != nil
    {
      screen = descriptor
      break
    }
  }
  if screen != nil { break }
  usleep(250_000)
}
guard let screen else { fail("device \(udid) exposes no display surface; is it booted?") }
var deviceSize = CGSize.zero
if let first = surfaceOf(screen) {
  deviceSize = CGSize(width: first.width, height: first.height)
  emit(["type": "ready", "width": first.width, "height": first.height])
}

// ── HID ─────────────────────────────────────────────────────────────────

typealias HIDInitFn = @convention(c) (AnyObject, Selector, AnyObject, UnsafeMutablePointer<NSError?>?) -> AnyObject?
typealias HIDSendFn = @convention(c) (
  AnyObject, Selector, UnsafeMutableRawPointer, Bool, AnyObject?, (@convention(block) (NSError?) -> Void)?
) -> Void

guard let hidClass = NSClassFromString("SimulatorKit.SimDeviceLegacyHIDClient") as? NSObject.Type
else { fail("SimDeviceLegacyHIDClient missing; unsupported Xcode") }
let initSel = NSSelectorFromString("initWithDevice:error:")
let sendSel = NSSelectorFromString("sendWithMessage:freeWhenDone:completionQueue:completion:")
var hidError: NSError?
guard
  let hid = unsafeBitCast(class_getMethodImplementation(hidClass, initSel), to: HIDInitFn.self)(
    hidClass.perform(NSSelectorFromString("alloc"))!.takeUnretainedValue(), initSel, device, &hidError)
else { fail("HID client: \(hidError?.localizedDescription ?? "unknown error")") }
let sendImp = unsafeBitCast(class_getMethodImplementation(object_getClass(hid), sendSel), to: HIDSendFn.self)
let hidQueue = DispatchQueue(label: "simbridge.hid")

/// Send one Indigo message (SimulatorKit frees it) and wait for delivery.
func send(_ message: UnsafeMutableRawPointer?) -> String? {
  // A nil message is a move SimulatorKit coalesced (<16ms after the last).
  guard let message else { return nil }
  var error: String?
  let done = DispatchSemaphore(value: 0)
  sendImp(hid, sendSel, message, true, hidQueue) { e in
    error = e?.localizedDescription
    done.signal()
  }
  done.wait()
  return error
}

// ── frames ──────────────────────────────────────────────────────────────

let ciContext = CIContext(options: [.cacheIntermediates: false])
let sRGB = CGColorSpace(name: CGColorSpace.sRGB)!
var lastSeed: UInt32 = 0
var seq: UInt64 = 0
var forceFrame = true

func captureFrame() {
  guard let surface = surfaceOf(screen) else { return }
  let seed = IOSurfaceGetSeed(surface)
  if seed == lastSeed && !forceFrame { return }
  lastSeed = seed
  forceFrame = false
  let size = CGSize(width: surface.width, height: surface.height)
  if size != deviceSize {
    deviceSize = size
    emit(["type": "ready", "width": Int(size.width), "height": Int(size.height)])
  }
  let scale = min(1, maxDim / max(size.width, size.height))
  let image = CIImage(ioSurface: surface).transformed(by: CGAffineTransform(scaleX: scale, y: scale))
  guard
    let jpeg = ciContext.jpegRepresentation(
      of: image, colorSpace: sRGB,
      options: [CIImageRepresentationOption(rawValue: kCGImageDestinationLossyCompressionQuality as String): quality])
  else { return }
  seq += 1
  emit([
    "type": "frame", "seq": seq,
    "width": Int((size.width * scale).rounded()), "height": Int((size.height * scale).rounded()),
    "device_width": Int(size.width), "device_height": Int(size.height),
    "data": jpeg.base64EncodedString(),
  ])
}

let frameQueue = DispatchQueue(label: "simbridge.frames")
let timer = DispatchSource.makeTimerSource(queue: frameQueue)
func schedule() {
  if fps <= 0 {
    timer.schedule(deadline: .distantFuture)
  } else {
    let interval = DispatchTimeInterval.microseconds(Int(1_000_000 / min(fps, 60)))
    timer.schedule(deadline: .now(), repeating: interval, leeway: .milliseconds(2))
  }
}
timer.setEventHandler(handler: captureFrame)
schedule()
timer.resume()

// ── commands ────────────────────────────────────────────────────────────

/// Home is a legacy button event; the side keys are the HID usages the device
/// chrome declares (/Library/Developer/DeviceKit/Chrome/*/chrome.json).
let legacyButtons: [String: Int32] = ["home": 0x0]
let hidButtons: [String: (page: UInt32, usage: UInt32)] = [
  "lock": (0x0c, 0x30),  // Consumer › Power (Sleep/Wake)
  "volume_up": (0x0c, 0xe9),
  "volume_down": (0x0c, 0xea),
  "action": (0x0b, 0x2d),  // Telephony › the Action button
]
let hardwareTarget: Int32 = 0x33
let touchTarget: UInt32 = 0x32

/// A touch that starts on the bottom edge carries SimulatorKit's bottom-edge
/// flag for its whole life: that is what makes the home-indicator swipe a
/// system gesture instead of a drag inside the app.
var gestureEdge: UInt32 = 0
let bottomEdge: UInt32 = 3

func touch(_ cmd: [String: Any]) -> String? {
  guard deviceSize.width > 0 else { return "no frame yet; the display is not ready" }
  let phase = cmd["phase"] as? String
  let eventType: UInt64
  switch phase {
  case "down": eventType = 1  // NSEventTypeLeftMouseDown
  case "move": eventType = 6  // NSEventTypeLeftMouseDragged
  case "up": eventType = 2  // NSEventTypeLeftMouseUp
  default: return "phase must be down, move or up"
  }
  func point(_ x: String, _ y: String) -> CGPoint? {
    guard let px = (cmd[x] as? NSNumber)?.doubleValue, let py = (cmd[y] as? NSNumber)?.doubleValue else { return nil }
    return CGPoint(x: min(max(px / deviceSize.width, 0), 1), y: min(max(py / deviceSize.height, 0), 1))
  }
  guard var first = point("x", "y") else { return "touch needs x and y" }
  var second = point("x2", "y2")
  if phase == "down" { gestureEdge = second == nil && first.y >= 0.985 ? bottomEdge : 0 }
  let edge = gestureEdge
  if phase == "up" { gestureEdge = 0 }
  let unit = CGSize(width: 1, height: 1)
  if second != nil {
    return send(mouseMessage(&first, &second!, touchTarget, eventType, edge, unit))
  }
  return send(mouseMessage(&first, nil, touchTarget, eventType, edge, unit))
}

func handle(_ cmd: [String: Any]) -> String? {
  switch cmd["op"] as? String {
  case "touch":
    return touch(cmd)
  case "button":
    let name = cmd["button"] as? String ?? ""
    let direction: Int32 = (cmd["phase"] as? String) == "up" ? 2 : 1
    if let source = legacyButtons[name] {
      return send(buttonMessage(source, direction, hardwareTarget))
    }
    guard let hid = hidButtons[name] else { return "unknown button '\(name)'" }
    return send(hidMessage(UInt32(hardwareTarget), hid.page, hid.usage, UInt32(direction)))
  case "key":
    guard let usage = (cmd["usage"] as? NSNumber)?.uint32Value else { return "key needs usage" }
    return send(keyMessage(usage, (cmd["phase"] as? String) == "up" ? 2 : 1))
  case "frame":
    frameQueue.async {
      forceFrame = true
      captureFrame()
    }
    return nil
  case "stream":
    frameQueue.async {
      fps = (cmd["fps"] as? NSNumber)?.doubleValue ?? fps
      maxDim = (cmd["max_dim"] as? NSNumber)?.doubleValue ?? maxDim
      quality = (cmd["quality"] as? NSNumber)?.doubleValue ?? quality
      forceFrame = true
      schedule()
    }
    return nil
  default:
    return "unknown op"
  }
}

// Input runs on its own thread so a slow encode never delays a touch.
Thread.detachNewThread {
  while let line = readLine(strippingNewline: true) {
    guard let data = line.data(using: .utf8),
      let cmd = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
    else { continue }
    let error = handle(cmd)
    if let id = cmd["id"] {
      var reply: [String: Any] = ["type": "done", "id": id]
      if let error { reply["error"] = error }
      emit(reply)
    }
  }
  // stdin closed: the worker is gone or stopped us.
  exit(0)
}

dispatchMain()
