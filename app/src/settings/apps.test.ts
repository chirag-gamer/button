import { describe, expect, it } from "vitest";
import { addApp, moveApp, removeApp } from "./apps";

describe("addApp", () => {
  it("appends a bare executable file name", () => {
    expect(addApp(["obs64.exe"], "Zoom.exe")).toEqual(["obs64.exe", "Zoom.exe"]);
  });

  it("rejects names without .exe, empty names and paths", () => {
    expect(addApp([], "")).toBeNull();
    expect(addApp([], "   ")).toBeNull();
    expect(addApp([], "Zoom")).toBeNull();
    expect(addApp([], ".exe")).toBeNull();
    expect(addApp([], "C:\\Tools\\Zoom.exe")).toBeNull();
    expect(addApp([], "sub/Zoom.exe")).toBeNull();
  });

  it("rejects a duplicate whatever the casing", () => {
    expect(addApp(["Zoom.exe"], "zoom.EXE")).toBeNull();
  });
});

describe("removeApp", () => {
  it("drops the entry case-insensitively", () => {
    expect(removeApp(["Zoom.exe", "obs64.exe"], "ZOOM.exe")).toEqual([
      "obs64.exe",
    ]);
  });

  it("leaves unrelated entries alone", () => {
    expect(removeApp(["Zoom.exe"], "obs64.exe")).toEqual(["Zoom.exe"]);
  });
});

describe("moveApp", () => {
  it("moves between the shelves with one pair of lists", () => {
    const next = moveApp(["Zoom.exe"], ["obs64.exe"], "zoom.exe");
    expect(next.from).toEqual([]);
    expect(next.to).toEqual(["obs64.exe", "Zoom.exe"]);
  });

  it("never lists an app twice on the target shelf", () => {
    const next = moveApp(["Zoom.exe"], ["zoom.EXE"], "Zoom.exe");
    expect(next.from).toEqual([]);
    expect(next.to).toEqual(["zoom.EXE"]);
  });
});
