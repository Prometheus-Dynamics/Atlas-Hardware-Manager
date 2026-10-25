import { describe, expect, test } from "bun:test";
import { streamLabel } from "./present.ts";

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
