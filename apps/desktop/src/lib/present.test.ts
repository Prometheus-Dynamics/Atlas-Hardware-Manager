import { describe, expect, test } from "bun:test";
import { bootEnd, streamLabel } from "./present.ts";

describe("streamLabel", () => {
  test("a raw view names its camera", () => {
    expect(streamLabel("front-raw")).toBe("Front · raw");
    expect(streamLabel("Arducam--B--raw")).toBe("Arducam B · raw");
  });
  test("a camera's own name reads as written", () => {
    expect(streamLabel("Front-Camera")).toBe("Front Camera");
    expect(streamLabel("camera")).toBe("Camera");
  });
  test("names HeliOS's raw camera names for people", () => {
    // As Raze #2 and the SystemCore listed them on the bench (2026-11-03).
    expect(streamLabel("ov9782--kernel-driver-i2c-10-0060-")).toBe("OV9782 (built-in)");
    expect(streamLabel("Creative-Live--Cam-Sync-V3--Cre")).toBe("Creative Live Cam Sync V3");
    expect(streamLabel("Streaming-Webcam--Streaming-Web")).toBe("Streaming Webcam");
    expect(streamLabel("Streaming-Webcam-Audio--Streami")).toBe("Streaming Webcam Audio");
    expect(streamLabel("046d-0825")).toBe("Logitech C270");
    expect(streamLabel("1bcf-0050")).toBe("USB camera 1bcf:0050");
    expect(streamLabel("C922-Pro-Stream-Webcam")).toBe("C922 Pro Stream Webcam");
    expect(streamLabel("capture_device_systemcore-3c99_video6")).toBe("Camera on /dev/video6");
  });
});

describe("bootEnd", () => {
  test("counts a power cut as normal and a warm restart without a shutdown as a crash", () => {
    expect(bootEnd("false", "power-on")).toBe("power");
    expect(bootEnd(false, null)).toBe("power");
    expect(bootEnd("false", "reset")).toBe("crash");
    expect(bootEnd("true", "reset")).toBe("clean");
    expect(bootEnd(null, "power-on")).toBe("unknown");
  });
});
