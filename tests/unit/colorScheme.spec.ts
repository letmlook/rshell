import { describe, expect, it } from "vitest";
import { parseColorScheme, parseColorSchemeText } from "../../src/utils/colorScheme";

const valid = {
  name: "My Scheme",
  ansi_colors: [
    0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5,
    0x7f7f7f, 0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
  ],
  default_fg: 0xe5e5e5,
  default_bg: 0x000000,
  cursor_fg: 0x000000,
  cursor_bg: 0xe5e5e5,
  selection_fg: 0x000000,
  selection_bg: 0x264f78,
};

describe("parseColorScheme", () => {
  it("accepts a well-formed scheme and trims the name", () => {
    const parsed = parseColorScheme({ ...valid, name: "  My Scheme  " });
    expect(parsed.name).toBe("My Scheme");
    expect(parsed.ansi_colors).toHaveLength(16);
    expect(parsed.default_fg).toBe(0xe5e5e5);
    expect(parsed.selection_bg).toBe(0x264f78);
  });

  it("rejects a non-object payload", () => {
    expect(() => parseColorScheme([])).toThrow(/JSON 对象/);
    expect(() => parseColorScheme("nope")).toThrow(/JSON 对象/);
    expect(() => parseColorScheme(null)).toThrow(/JSON 对象/);
  });

  it("names the missing name field", () => {
    const { name, ...withoutName } = valid;
    void name;
    expect(() => parseColorScheme(withoutName)).toThrow(/「name」/);
  });

  it("rejects an empty name", () => {
    expect(() => parseColorScheme({ ...valid, name: "   " })).toThrow(/「name」/);
  });

  it("requires exactly 16 ansi colors", () => {
    expect(() => parseColorScheme({ ...valid, ansi_colors: [0, 1, 2] })).toThrow(
      /必须有 16 项，实际 3 项/,
    );
  });

  it("names the offending ansi index when a color is out of range", () => {
    const ansi = [...valid.ansi_colors] as number[];
    ansi[7] = 0x1000000;
    expect(() => parseColorScheme({ ...valid, ansi_colors: ansi })).toThrow(
      /「ansi_colors\[7\]」/,
    );
  });

  it("names the offending field when a color is not a number", () => {
    expect(() => parseColorScheme({ ...valid, cursor_bg: "red" })).toThrow(/「cursor_bg」/);
  });

  it("rejects negative colors", () => {
    expect(() => parseColorScheme({ ...valid, default_bg: -1 })).toThrow(/「default_bg」/);
  });
});

describe("parseColorSchemeText", () => {
  it("parses JSON text", () => {
    expect(parseColorSchemeText(JSON.stringify(valid)).name).toBe("My Scheme");
  });

  it("reports invalid JSON instead of throwing a raw parser error", () => {
    expect(() => parseColorSchemeText("{not json")).toThrow(/不是合法的 JSON/);
  });
});
