import { describe, expect, it } from "vitest";
import { formatElapsed } from "./format";

describe("formatElapsed", () => {
  it("formats under an hour as m:ss", () => {
    expect(formatElapsed(0)).toBe("0:00");
    expect(formatElapsed(5)).toBe("0:05");
    expect(formatElapsed(59)).toBe("0:59");
    expect(formatElapsed(60)).toBe("1:00");
    expect(formatElapsed(3599)).toBe("59:59");
  });

  it("formats an hour or more as h:mm:ss", () => {
    expect(formatElapsed(3600)).toBe("1:00:00");
    expect(formatElapsed(3661)).toBe("1:01:01");
  });

  it("treats negative time as zero", () => {
    expect(formatElapsed(-3)).toBe("0:00");
  });
});
