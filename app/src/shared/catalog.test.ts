import { describe, expect, it } from "vitest";
import settingsRs from "../../src-tauri/src/settings.rs?raw";
import { BLUR_PRESETS, BUILTIN_BACKDROPS } from "./catalog";

describe("built-in backdrops", () => {
  it("match the ids settings.rs validates, in order", () => {
    const block = /BUILTIN_BACKDROPS: \[&str; \d+\] = \[([^\]]*)\]/.exec(
      settingsRs,
    );
    expect(block).not.toBeNull();
    const rustIds = [...block![1].matchAll(/"([a-z-]+)"/g)].map((m) => m[1]);
    expect(BUILTIN_BACKDROPS.map((b) => b.id)).toEqual(rustIds);
  });

  it("keep the blur presets inside the validated 0-40 px", () => {
    for (const preset of BLUR_PRESETS) {
      expect(preset.px).toBeGreaterThanOrEqual(0);
      expect(preset.px).toBeLessThanOrEqual(40);
    }
    // settings.rs DEFAULT_BLUR is the "Medium" preset.
    const medium = BLUR_PRESETS.find((p) => p.label === "Medium")!;
    expect(settingsRs).toContain(`DEFAULT_BLUR: u8 = ${medium.px};`);
  });
});
