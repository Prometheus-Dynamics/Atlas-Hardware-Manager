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
