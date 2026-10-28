// eventToAccelerator: KeyboardEvent → accelerator token (FR-45).
import { describe, expect, it } from "vitest";
import { eventToAccelerator, isCapturableCode } from "./accel";

const ev = (overrides: Partial<KeyboardEvent>) => ({
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  code: "KeyA",
  key: "a",
  ...overrides,
});

describe("eventToAccelerator", () => {
  it("maps a plain letter with modifiers in order", () => {
    expect(
      eventToAccelerator(ev({ ctrlKey: true, altKey: true, code: "KeyK" })),
    ).toBe("Ctrl+Alt+K");
  });

  it("spells the Win key Super", () => {
    expect(eventToAccelerator(ev({ metaKey: true, code: "KeyP" }))).toBe(
      "Super+P",
    );
  });

  it("uses the code for shifted digits", () => {
    expect(
      eventToAccelerator(ev({ shiftKey: true, code: "Digit1", key: "!" })),
    ).toBe("Shift+1");
  });

  it("maps Space and arrows to parser names", () => {
    expect(eventToAccelerator(ev({ code: "Space", key: " " }))).toBe("Space");
    expect(eventToAccelerator(ev({ code: "ArrowLeft", key: "Left" }))).toBe(
      "Left",
    );
  });

  it("maps function keys", () => {
    expect(eventToAccelerator(ev({ code: "F12", key: "F12" }))).toBe("F12");
    expect(eventToAccelerator(ev({ code: "F24", key: "F24" }))).toBe("F24");
  });

  it("returns null for modifier-only presses", () => {
    expect(eventToAccelerator(ev({ ctrlKey: true, code: "ControlLeft" }))).toBe(
      null,
    );
    expect(eventToAccelerator(ev({ code: "ShiftLeft", key: "Shift" }))).toBe(
      null,
    );
  });

  it("returns null for uncapturable keys", () => {
    expect(eventToAccelerator(ev({ code: "Semicolon", key: ";" }))).toBe(null);
  });

  it("returns Escape as a key so callers can cancel with it", () => {
    expect(eventToAccelerator(ev({ code: "Escape", key: "Escape" }))).toBe(
      "Escape",
    );
  });
});

describe("isCapturableCode", () => {
  it("accepts letters, digits, f-keys and named keys", () => {
    expect(isCapturableCode("KeyZ")).toBe(true);
    expect(isCapturableCode("Digit0")).toBe(true);
    expect(isCapturableCode("F1")).toBe(true);
    expect(isCapturableCode("F25")).toBe(false);
    expect(isCapturableCode("Semicolon")).toBe(false);
  });
});
