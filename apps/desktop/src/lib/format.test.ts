import { describe, expect, test } from "bun:test";

import { compareVersionsDesc } from "./format.ts";

describe("compareVersionsDesc", () => {
  test("dev builds order by count, below their release, ignoring the commit", () => {
    const sorted = ["2026.1.0-dev.99+ffffffff", "2026.0.0", "2026.1.0", "2026.1.0-dev.100+00000000"].sort(
      compareVersionsDesc,
    );
    expect(sorted).toEqual(["2026.1.0", "2026.1.0-dev.100+00000000", "2026.1.0-dev.99+ffffffff", "2026.0.0"]);
    expect(compareVersionsDesc("2026.1.0+abc", "2026.1.0")).toBe(0);
  });
});
