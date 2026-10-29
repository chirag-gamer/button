import { describe, expect, it } from "vitest";
import { moveBetween, reorder, Shelves } from "./shelves";

describe("moveBetween", () => {
  const shelves: Shelves = {
    shown: ["keep-awake", "microphone"],
    more: ["dnd", "taskbar"],
    hidden: [],
  };

  it("moves an id from another shelf onto the target", () => {
    const next = moveBetween(shelves, "dnd", "shown");
    expect(next.shown).toEqual(["keep-awake", "microphone", "dnd"]);
    expect(next.more).toEqual(["taskbar"]);
    expect(next.hidden).toEqual([]);
  });

  it("moves an id to the hidden shelf", () => {
    const next = moveBetween(shelves, "microphone", "hidden");
    expect(next.shown).toEqual(["keep-awake"]);
    expect(next.hidden).toEqual(["microphone"]);
  });

  it("leaves the shelf alone when the id is already there", () => {
    const next = moveBetween(shelves, "keep-awake", "shown");
    expect(next.shown).toEqual(["keep-awake", "microphone"]);
    expect(next.more).toEqual(["dnd", "taskbar"]);
  });

  it("does not mutate the input", () => {
    moveBetween(shelves, "dnd", "shown");
    expect(shelves.shown).toEqual(["keep-awake", "microphone"]);
    expect(shelves.more).toEqual(["dnd", "taskbar"]);
  });
});

describe("reorder", () => {
  const list = ["a", "b", "c"];

  it("swaps a row with its neighbour", () => {
    expect(reorder(list, 1, -1)).toEqual(["b", "a", "c"]);
    expect(reorder(list, 1, 1)).toEqual(["a", "c", "b"]);
  });

  it("clamps at both ends", () => {
    expect(reorder(list, 0, -1)).toEqual(list);
    expect(reorder(list, 2, 1)).toEqual(list);
  });

  it("returns a copy so callers can store it as settings", () => {
    const next = reorder(list, 2, 1);
    expect(next).toEqual(list);
    expect(next).not.toBe(list);
  });
});
