import { expect, test } from "bun:test";
import { pipelinesOf } from "./nt";

test("two cameras' pipelines of one name are told apart", () => {
  const n = (value: number) => ({ value: { type: "double", value } }) as never;
  const topics = new Map([
    ["/HeliOS/cam1/default/fps", n(30)],
    ["/HeliOS/cam1/default/latency_ms", n(4)],
    ["/HeliOS/cam2/default/fps", n(60)],
    ["/HeliOS/cam2/default/latency_ms", n(3)],
    ["/HeliOS/cam2/tags/fps", n(15)],
    ["/HeliOS/cam2/tags/latency_ms", n(9)],
  ]);
  const shown = pipelinesOf(topics).map((p) => [p.path, p.name]);
  expect(shown).toEqual([
    ["/HeliOS/cam1/default/", "cam1/default"],
    ["/HeliOS/cam2/default/", "cam2/default"],
    ["/HeliOS/cam2/tags/", "tags"],
  ]);
});
